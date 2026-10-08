import * as vscode from "vscode";
import { ensureCodingAgentReady } from "./agentReadiness";
import {
  ActivityPhase,
  ActivityReporter,
  OpenMindChatPanel,
  OpenMindChatViewProvider,
  SYSTEM_PROMPT,
  buildWorkspaceRequest,
  chatWithEditRepair,
  deliverAnswer
} from "./chatPanel";
import { NativeInterfaceManager } from "./nativeInterface";
import {
  ChatMessage,
  DEFAULT_MAX_OUTPUT_TOKENS,
  OpenMindClient,
  RequestProfile,
  resolveMaxOutputTokens
} from "./openmindClient";
import {
  RuntimeTarget,
  codingAgentHeaders,
  describeTarget,
  findCodingAgentEndpoint,
  isNativeInterfaceEndpoint
} from "./runtimeDiscovery";

const output = vscode.window.createOutputChannel("OpenMindAI");
const nativeInterface = new NativeInterfaceManager(output);
let discoveredTarget: RuntimeTarget | undefined;
// Capability probes for manually configured endpoints, keyed by endpoint.
const configuredTargets = new Map<string, RuntimeTarget>();
// Optional observer of what the extension shows users. Unset in normal use.
let eventSink: ((kind: string, text: string) => void) | undefined;

function emitEvent(kind: string, text: string): void {
  eventSink?.(kind, text);
}

let confirmFix = (): Thenable<string | undefined> =>
  vscode.window.showWarningMessage(
    "Apply OpenMindAI's proposed replacement to the selected range?",
    { modal: true },
    "Apply",
    "Keep Preview"
  );

class PreviewContentProvider implements vscode.TextDocumentContentProvider {
  private readonly contents = new Map<string, string>();
  private readonly emitter = new vscode.EventEmitter<vscode.Uri>();

  readonly onDidChange = this.emitter.event;

  provideTextDocumentContent(uri: vscode.Uri): string {
    return this.contents.get(uri.toString()) ?? "";
  }

  set(uri: vscode.Uri, content: string): void {
    this.contents.set(uri.toString(), content);
    this.emitter.fire(uri);
  }
}

export function activate(context: vscode.ExtensionContext): unknown {
  const previewProvider = new PreviewContentProvider();
  const viewProvider = new OpenMindChatViewProvider(context, createClient, prepareForChatSurface);
  context.subscriptions.push(
    vscode.workspace.registerTextDocumentContentProvider("openmindai-preview", previewProvider),
    output,
    registerChatParticipant(context),
    vscode.window.registerWebviewViewProvider(
      OpenMindChatViewProvider.viewType,
      viewProvider,
      {
        webviewOptions: {
          retainContextWhenHidden: true
        }
      }
    )
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("openmindai.openChat", () => {
      void openRightSideChat(context);
    }),
    vscode.commands.registerCommand("openmindai.openAgentChat", () => {
      void openNativeChat(context);
    }),
    vscode.commands.registerCommand("openmindai.startNativeInterface", startNativeInterface),
    vscode.commands.registerCommand("openmindai.stopNativeInterface", stopNativeInterface),
    vscode.commands.registerCommand("openmindai.buildNativeInterface", buildNativeInterface),
    vscode.commands.registerCommand("openmindai.checkConnection", checkConnection),
    vscode.commands.registerCommand("openmindai.ask", askOpenMindAI),
    vscode.commands.registerCommand("openmindai.explainSelection", explainSelection),
    vscode.commands.registerCommand("openmindai.fixSelection", () => fixSelection(previewProvider))
  );

  if (context.extensionMode !== vscode.ExtensionMode.Test) return undefined;
  // Integration-test harness: a separate module that is not shipped in the VSIX and is only
  // loaded when VS Code runs the extension under --extensionTestsPath.
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const harness = require("./testHooks") as typeof import("./testHooks");
  return harness.installTestHooks({
    viewProvider,
    openPanel: () => OpenMindChatPanel.createOrShow(context, createClient, prepareForChatSurface),
    activePanel: () => OpenMindChatPanel.active,
    runParticipant: (prompt, output) => handleParticipantRequest(prompt, output, () => false),
    target: () => activeTarget(),
    setEventSink: (sink) => {
      eventSink = sink;
    },
    setFixConfirm: (confirm) => {
      confirmFix = confirm;
    }
  });
}

export function deactivate(): void {
  // No long-running extension resources are kept outside VS Code subscriptions.
}

function configuredEndpoint(): string | undefined {
  const value = vscode.workspace.getConfiguration("openmindai").get<string>("endpoint", "auto").trim();
  return value && value.toLowerCase() !== "auto" ? value.replace(/\/+$/, "") : undefined;
}

function activeTarget(): RuntimeTarget | undefined {
  const manual = configuredEndpoint();
  if (!manual) return discoveredTarget;
  return (
    configuredTargets.get(manual) ?? {
      endpoint: manual,
      kind: isNativeInterfaceEndpoint(manual) ? "native-gateway" : "custom"
    }
  );
}

const PROFILE_BY_KIND: Record<RuntimeTarget["kind"], RequestProfile> = {
  "coding-agent": "coding-agent",
  "llama-server": "llama-server",
  "native-gateway": "native-gateway",
  // Unknown OpenAI-compatible servers get llama.cpp fields; strict servers that reject them
  // are detected on the first 400 and retried without extensions.
  custom: "llama-server"
};

function createClient(): OpenMindClient {
  const config = vscode.workspace.getConfiguration("openmindai");
  const target = activeTarget();
  if (!target) {
    throw new Error("OpenMindAI endpoint was not found. Start the desktop app or set openmindai.endpoint.");
  }
  const profile = PROFILE_BY_KIND[target.kind];
  let maxOutputTokens = resolveMaxOutputTokens(
    config.get<number>("maxOutputTokens", DEFAULT_MAX_OUTPUT_TOKENS)
  );
  let contextSize: number | undefined;
  if (profile === "native-gateway") {
    // The native worker rejects prompt + max_tokens beyond its context window, so leave
    // room for the prompt inside openmindai.nativeContextSize.
    contextSize = config.get<number>("nativeContextSize", 1024);
    maxOutputTokens = Math.min(maxOutputTokens, resolveMaxOutputTokens(Math.floor(contextSize / 2)));
  }
  return new OpenMindClient({
    endpoint: target.endpoint,
    model: config.get<string>("model", "openmind-local"),
    requestTimeoutMs: config.get<number>("requestTimeoutMs", 300000),
    maxOutputTokens,
    apiToken: config.get<string>("apiToken", ""),
    profile,
    contextSize,
    label: describeTarget(target),
    extraHeaders: target.kind === "coding-agent" ? codingAgentHeaders(target.token) : undefined
  });
}

/**
 * Finds the endpoint and, for the desktop coding agent, waits until the Agent Setup model is
 * loaded. Progress (for example "OpenMindAI is loading OpenAgent Lite… 42s") goes to `progress`.
 */
async function prepareForRequest(progress: (message: string) => void): Promise<void> {
  await prepareLocalEndpoint();
  const target = activeTarget();
  if (target?.kind !== "coding-agent") return;
  target.agent = await ensureCodingAgentReady(target.endpoint, target.token, (message) => {
    emitEvent("progress", message);
    progress(message);
  });
}

function prepareForChatSurface(report: ActivityReporter): Promise<void> {
  return prepareForRequest((message) => report("Thinking", message));
}

async function prepareLocalEndpoint(): Promise<void> {
  const manual = configuredEndpoint();
  if (manual) {
    if (!configuredTargets.has(manual)) {
      const target = await probeConfiguredEndpoint(manual);
      configuredTargets.set(manual, target);
      output.appendLine(`Using configured endpoint as ${describeTarget(target)}`);
    }
    return;
  }
  // Replace, never keep, the previous target: after a desktop restart the old endpoint and
  // session token are gone, and reusing them would only produce connection errors.
  discoveredTarget = await nativeInterface.ensureRunning();
}

/** Identifies what a manually configured endpoint is so only supported fields are sent. */
async function probeConfiguredEndpoint(endpoint: string): Promise<RuntimeTarget> {
  if (isNativeInterfaceEndpoint(endpoint)) return { endpoint, kind: "native-gateway" };
  // The coding agent needs the session token from the desktop descriptor.
  const agent = await findCodingAgentEndpoint();
  if (agent && agent.endpoint === endpoint) return agent;
  return { endpoint, kind: "custom" };
}

export interface ParticipantOutput {
  progress(message: string): void;
  markdown(text: string): void;
}

async function handleParticipantRequest(
  prompt: string,
  response: ParticipantOutput,
  isCancelled: () => boolean
): Promise<void> {
  const report = (phase: ActivityPhase, detail?: string): void => {
    response.progress(detail ? `${phase}: ${detail}` : phase);
  };

  try {
    report("Planning", "Preparing workspace context");
    const workspaceRequest = await buildWorkspaceRequest(prompt, report);
    if (isCancelled()) return;

    await prepareForRequest((message) => report("Thinking", message));
    if (isCancelled()) return;

    const client = createClient();
    report("Thinking", `Waiting for ${client.label}`);
    const answer = await chatWithEditRepair(
      client,
      [
        { role: "system", content: SYSTEM_PROMPT },
        { role: "user", content: workspaceRequest }
      ],
      prompt,
      report
    );
    if (isCancelled()) return;

    const applied = await deliverAnswer(answer, report, (text) => response.markdown(text), prompt);
    if (applied.length) {
      const changed = applied.map((file) => `- \`${file}\``).join("\n");
      response.markdown(`\n\nApplied edits:\n${changed}`);
    }
    report("Done", applied.length ? "Workspace updated" : "Response ready");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    report("Error", message);
    response.markdown(`OpenMindAI error: ${message}`);
  }
}

function registerChatParticipant(context: vscode.ExtensionContext): vscode.Disposable {
  const participant = vscode.chat.createChatParticipant(
    "openmindai.agent",
    async (request, _chatContext, response, token) => {
      emitEvent("participant", request.prompt);
      await handleParticipantRequest(
        request.prompt,
        {
          progress: (text) => {
            emitEvent("participant-progress", text);
            response.progress(text);
          },
          markdown: (text) => {
            emitEvent("participant-markdown", text);
            response.markdown(text);
          }
        },
        () => token.isCancellationRequested
      );
    }
  );

  participant.iconPath = vscode.Uri.joinPath(context.extensionUri, "media", "icon.png");
  participant.followupProvider = {
    provideFollowups: () => [
      { prompt: "Explain the active file", label: "Explain active file" },
      { prompt: "Audit this workspace and report risks", label: "Audit workspace" },
      { prompt: "Fix the selected code", label: "Fix selected code" }
    ]
  };
  return participant;
}

async function openNativeChat(context: vscode.ExtensionContext): Promise<void> {
  try {
    await vscode.commands.executeCommand("workbench.action.chat.open", { query: "@openmindai " });
  } catch (error) {
    output.appendLine(
      `[${new Date().toISOString()}] Could not open native chat, falling back to webview: ${
        error instanceof Error ? error.message : String(error)
      }`
    );
    OpenMindChatPanel.createOrShow(context, createClient, prepareForChatSurface);
  }
}

async function openRightSideChat(context: vscode.ExtensionContext): Promise<void> {
  try {
    await vscode.commands.executeCommand("openmindai.chatView.focus");
  } catch (error) {
    output.appendLine(
      `[${new Date().toISOString()}] Could not focus right-side chat, falling back to webview panel: ${
        error instanceof Error ? error.message : String(error)
      }`
    );
    OpenMindChatPanel.createOrShow(context, createClient, prepareForChatSurface);
  }
}

async function checkConnection(): Promise<void> {
  try {
    await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Notification,
        title: "Checking OpenMindAI local connection..."
      },
      async () => {
        await prepareLocalEndpoint();
        const client = createClient();
        const health = await client.health();
        const ready = await client.ready();
        const target = activeTarget();
        emitEvent("connection", `${target?.kind ?? "none"}: ${client.label}`);
        if (target?.kind === "coding-agent" && target.agent?.configured === false) {
          vscode.window.showWarningMessage(
            `OpenMindAI is reachable, but ${target.agent.message ?? "no coding agent is configured."}`
          );
          return;
        }
        if (target?.kind === "coding-agent" && target.agent?.codingEnabled === false) {
          vscode.window.showWarningMessage(
            "OpenMindAI is reachable, but Coding Workspace is disabled in Settings -> Agent Setup, so coding requests are unavailable."
          );
          return;
        }
        if (target?.kind === "llama-server") {
          vscode.window.showWarningMessage(
            `OpenMindAI is reachable through the ${client.label}. Update the desktop app so VS Code uses the Agent Setup coding agent.`
          );
          return;
        }
        vscode.window.showInformationMessage(
          `OpenMindAI is reachable via ${client.label}. health=${compactJson(health)} ready=${compactJson(ready)}`
        );
      }
    );
  } catch (error) {
    showError("OpenMindAI connection check failed", error);
  }
}

async function startNativeInterface(): Promise<void> {
  try {
    await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Notification,
        title: "Starting OpenMindAI native interface..."
      },
      () => nativeInterface.start()
    );
    vscode.window.showInformationMessage("OpenMindAI native interface is ready.");
  } catch (error) {
    showError("OpenMindAI native interface start failed", error);
  }
}

async function stopNativeInterface(): Promise<void> {
  try {
    await nativeInterface.stop();
    vscode.window.showInformationMessage("OpenMindAI native interface stopped.");
  } catch (error) {
    showError("OpenMindAI native interface stop failed", error);
  }
}

async function buildNativeInterface(): Promise<void> {
  try {
    await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Notification,
        title: "Building OpenMindAI native interface..."
      },
      () => nativeInterface.build()
    );
    vscode.window.showInformationMessage("OpenMindAI native interface built.");
  } catch (error) {
    showError("OpenMindAI native interface build failed", error);
  }
}

/** `promptArg` lets keybindings and other commands pass the question directly. */
async function askOpenMindAI(promptArg?: unknown): Promise<void> {
  const prompt =
    typeof promptArg === "string" && promptArg.trim()
      ? promptArg
      : await vscode.window.showInputBox({
          title: "Ask OpenMindAI",
          prompt: "Ask about the active file or project",
          ignoreFocusOut: true
        });
  if (!prompt?.trim()) return;

  const context = activeEditorContext();
  const messages: ChatMessage[] = [
    { role: "system", content: SYSTEM_PROMPT },
    {
      role: "user",
      content: `${context}\n\nUser request:\n${prompt.trim()}`
    }
  ];
  const answer = await runChat(messages, "Asking OpenMindAI...");
  if (answer) await showMarkdown("OpenMindAI Answer", answer);
}

async function explainSelection(): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.selection.isEmpty) {
    vscode.window.showWarningMessage("Select code to explain first.");
    return;
  }

  const selected = editor.document.getText(editor.selection);
  const language = editor.document.languageId;
  const messages: ChatMessage[] = [
    { role: "system", content: SYSTEM_PROMPT },
    {
      role: "user",
      content: `Explain this ${language} code clearly and call out important behavior or risks.\n\n\`\`\`${language}\n${selected}\n\`\`\``
    }
  ];
  const answer = await runChat(messages, "Explaining selection...");
  if (answer) await showMarkdown("OpenMindAI Explanation", answer);
}

async function fixSelection(previewProvider: PreviewContentProvider): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.selection.isEmpty) {
    vscode.window.showWarningMessage("Select code to fix first.");
    return;
  }

  const document = editor.document;
  const selected = document.getText(editor.selection);
  const language = document.languageId;
  const diagnostics = vscode.languages
    .getDiagnostics(document.uri)
    .filter((item) => editor.selection.contains(item.range.start) || editor.selection.intersection(item.range));

  const messages: ChatMessage[] = [
    {
      role: "system",
      content:
        "You are OpenMindAI inside VS Code. Return only replacement code for the selected range. Do not include markdown fences or explanation."
    },
    {
      role: "user",
      content: [
        `Fix or improve this ${language} selection while preserving intent.`,
        diagnostics.length ? `Diagnostics:\n${diagnostics.map(formatDiagnostic).join("\n")}` : "Diagnostics: none in selection.",
        `Selected code:\n\`\`\`${language}\n${selected}\n\`\`\``
      ].join("\n\n")
    }
  ];

  const rawAnswer = await runChat(messages, "Generating proposed fix...");
  if (!rawAnswer) return;
  const replacement = extractCode(rawAnswer).trimEnd();
  if (!replacement) {
    vscode.window.showWarningMessage("OpenMindAI did not return replacement code.");
    return;
  }

  await showDiff(previewProvider, selected, replacement, language);
  emitEvent("fix-preview", replacement);
  const choice = await confirmFix();
  if (choice !== "Apply") return;

  const edit = new vscode.WorkspaceEdit();
  edit.replace(document.uri, editor.selection, replacement);
  const applied = await vscode.workspace.applyEdit(edit);
  if (applied) {
    vscode.window.showInformationMessage("OpenMindAI fix applied.");
  } else {
    vscode.window.showErrorMessage("VS Code could not apply the OpenMindAI fix.");
  }
}

async function runChat(messages: ChatMessage[], title: string): Promise<string | undefined> {
  try {
    return await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Notification,
        title
      },
      async (progress) => {
        await prepareForRequest((message) => progress.report({ message }));
        const client = createClient();
        progress.report({ message: `Waiting for ${client.label}` });
        return client.chat(messages);
      }
    );
  } catch (error) {
    showError(title.replace(/\.\.\.$/, " failed"), error);
    return undefined;
  }
}

function activeEditorContext(): string {
  const editor = vscode.window.activeTextEditor;
  if (!editor) return "No active editor.";

  const document = editor.document;
  const selected = editor.selection.isEmpty ? "" : document.getText(editor.selection);
  if (selected) {
    return `Active file: ${document.uri.fsPath}\nLanguage: ${document.languageId}\nSelected text:\n\`\`\`${document.languageId}\n${selected}\n\`\`\``;
  }

  const maxContext = vscode.workspace
    .getConfiguration("openmindai")
    .get<number>("maxContextCharacters", 800);
  const fullText = document.getText();
  const preview = fullText.length > maxContext ? `${fullText.slice(0, maxContext)}\n[truncated]` : fullText;
  return `Active file: ${document.uri.fsPath}\nLanguage: ${document.languageId}\nFile preview:\n\`\`\`${document.languageId}\n${preview}\n\`\`\``;
}

async function showMarkdown(title: string, content: string): Promise<void> {
  emitEvent("markdown", `${title}\n${content}`);
  const doc = await vscode.workspace.openTextDocument({
    language: "markdown",
    content: `# ${title}\n\n${content}`
  });
  await vscode.window.showTextDocument(doc, vscode.ViewColumn.Beside);
}

async function showDiff(
  previewProvider: PreviewContentProvider,
  before: string,
  after: string,
  language: string
): Promise<void> {
  const id = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  const beforeUri = vscode.Uri.parse(`openmindai-preview:/before-${id}.${extensionForLanguage(language)}`);
  const afterUri = vscode.Uri.parse(`openmindai-preview:/after-${id}.${extensionForLanguage(language)}`);
  previewProvider.set(beforeUri, before);
  previewProvider.set(afterUri, after);
  await vscode.commands.executeCommand("vscode.diff", beforeUri, afterUri, "OpenMindAI Proposed Fix");
}

function formatDiagnostic(diagnostic: vscode.Diagnostic): string {
  const line = diagnostic.range.start.line + 1;
  const character = diagnostic.range.start.character + 1;
  return `- ${line}:${character} ${diagnostic.message}`;
}

function extractCode(value: string): string {
  const fence = value.match(/```[a-zA-Z0-9_-]*\s*([\s\S]*?)```/);
  return fence?.[1] ?? value;
}

function compactJson(value: unknown): string {
  return JSON.stringify(value).replace(/\s+/g, " ");
}

function showError(label: string, error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  output.appendLine(`[${new Date().toISOString()}] ${label}: ${message}`);
  emitEvent("error", `${label}: ${message}`);
  vscode.window.showErrorMessage(`${label}: ${message}`);
}

function extensionForLanguage(language: string): string {
  const map: Record<string, string> = {
    javascript: "js",
    typescript: "ts",
    javascriptreact: "jsx",
    typescriptreact: "tsx",
    python: "py",
    rust: "rs",
    go: "go",
    json: "json",
    markdown: "md",
    html: "html",
    css: "css"
  };
  return map[language] ?? "txt";
}
