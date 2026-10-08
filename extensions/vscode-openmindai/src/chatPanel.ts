import * as vscode from "vscode";
import * as path from "path";
import {
  EDIT_FENCE_TAG,
  EDIT_PLAN_EXAMPLE,
  EDIT_REPAIR_PROMPT,
  EditPlan,
  EditTarget,
  assertInsideRealWorkspace,
  UNREQUESTED_EDIT_NOTE,
  extractEditPlan,
  looksLikeEditRequest,
  needsEditRepair,
  resolveEditTargets,
  stripEditFences
} from "./editProtocol";
import { ChatMessage, OpenMindClient } from "./openmindClient";

export const SYSTEM_PROMPT = [
  "You are OpenMindAI, the local coding agent inside VS Code. Be concise, practical, and careful with code.",
  "For questions and explanations, answer in plain prose or Markdown and never output an edit block.",
  `When the user asks you to create or change workspace files, reply with exactly one fenced code block tagged ${EDIT_FENCE_TAG} and nothing else.`,
  "Return only that edit payload: no explanation before or after it, and no other Markdown.",
  `The block must contain JSON in this exact shape: ${EDIT_PLAN_EXAMPLE}`,
  "Put a one-line description of the change in summary. content is the complete new file content, not a diff.",
  "Only include files that must change. Use paths relative to the workspace root. Never use absolute paths or parent-directory traversal."
].join(" ");

export type ActivityPhase = "Planning" | "Reading" | "Thinking" | "Writing" | "Done" | "Error";
export type ActivityReporter = (phase: ActivityPhase, detail?: string) => void;
/** Runs before each request; may report progress (for example while the agent model loads). */
export type BeforeRequest = (report: ActivityReporter) => Promise<void>;

/** Optional observer of a chat surface (used by development tooling). */
export interface ChatSurfaceObserver {
  /** Returns true when it consumed a webview message. */
  receive(message: unknown): boolean;
  busyChanged(busy: boolean): void;
}

let extraWebviewScript = "";

/** Script appended to the chat webview page. Empty in normal use. */
export function setExtraWebviewScript(script: string): void {
  extraWebviewScript = script;
}

/**
 * Sends a workspace request. If an edit request comes back as code without an edit plan
 * (small models sometimes answer with a plain code block), asks once for the canonical
 * openmindai-edits format. The repaired answer still goes through full edit validation; if
 * it is still not a valid plan, the original answer is shown as text.
 */
export async function chatWithEditRepair(
  client: OpenMindClient,
  messages: ChatMessage[],
  userText: string,
  report?: ActivityReporter
): Promise<string> {
  const answer = await client.chat(messages);
  if (!needsEditRepair(userText, answer)) return answer;
  report?.("Thinking", "Asking the agent to resend the change in the OpenMindAI edit format");
  const repaired = await client.chat([
    ...messages,
    { role: "assistant", content: answer },
    { role: "user", content: EDIT_REPAIR_PROMPT }
  ]);
  return needsEditRepair(userText, repaired) ? answer : repaired;
}

/** One chat turn shared by the right-side view and the editor panel. */
async function runChatTurn(
  text: string,
  messages: ChatMessage[],
  post: (message: unknown) => void,
  clientFactory: () => OpenMindClient,
  beforeRequest?: BeforeRequest
): Promise<void> {
  const report: ActivityReporter = (phase, detail) => post({ type: "activity", phase, detail });
  report("Planning", "Preparing workspace context");
  const request = await buildWorkspaceRequest(text, report);
  messages.push({ role: "user", content: request });
  post({ type: "append", role: "user", content: text });
  post({ type: "busy", busy: true });

  try {
    await beforeRequest?.(report);
    const client = clientFactory();
    report("Thinking", `Waiting for ${client.label}`);
    const answer = await chatWithEditRepair(client, messages, text, report);
    messages.push({ role: "assistant", content: answer });
    const applied = await deliverAnswer(
      answer,
      report,
      (text) => post({ type: "append", role: "assistant", content: text }),
      text
    );
    if (applied.length) {
      post({ type: "append", role: "OpenMindAI", content: `Applied edits:\n${applied.map((file) => `- ${file}`).join("\n")}` });
    }
    report("Done", applied.length ? "Workspace updated" : "Response ready");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    report("Error", message);
    post({ type: "append", role: "assistant", content: `OpenMindAI error: ${message}` });
  } finally {
    post({ type: "busy", busy: false });
  }
}

export class OpenMindChatPanel {
  private static current: OpenMindChatPanel | undefined;

  private readonly messages: ChatMessage[] = [{ role: "system", content: SYSTEM_PROMPT }];
  private readonly disposables: vscode.Disposable[] = [];
  observer: ChatSurfaceObserver | undefined;

  static get active(): OpenMindChatPanel | undefined {
    return OpenMindChatPanel.current;
  }

  static createOrShow(
    context: vscode.ExtensionContext,
    clientFactory: () => OpenMindClient,
    beforeRequest?: BeforeRequest
  ): OpenMindChatPanel {
    if (OpenMindChatPanel.current) {
      OpenMindChatPanel.current.panel.reveal(vscode.ViewColumn.Beside);
      return OpenMindChatPanel.current;
    }

    const panel = vscode.window.createWebviewPanel(
      "openmindai.chat",
      "OpenMindAI",
      vscode.ViewColumn.Beside,
      {
        enableScripts: true,
        localResourceRoots: [context.extensionUri]
      }
    );
    OpenMindChatPanel.current = new OpenMindChatPanel(panel, clientFactory, beforeRequest);
    return OpenMindChatPanel.current;
  }

  private constructor(
    private readonly panel: vscode.WebviewPanel,
    private readonly clientFactory: () => OpenMindClient,
    private readonly beforeRequest?: BeforeRequest
  ) {
    this.panel.webview.html = this.html(this.panel.webview);
    this.panel.onDidDispose(() => this.dispose(), null, this.disposables);
    this.panel.webview.onDidReceiveMessage(
      async (message: { type?: string; text?: string }) => {
        if (this.observer?.receive(message)) return;
        if (message.type === "ask" && message.text?.trim()) {
          await this.ask(message.text.trim());
        }
      },
      null,
      this.disposables
    );
  }

  dispose(): void {
    OpenMindChatPanel.current = undefined;
    while (this.disposables.length) {
      this.disposables.pop()?.dispose();
    }
  }

  private async ask(text: string): Promise<void> {
    this.observer?.busyChanged(true);
    try {
      await runChatTurn(text, this.messages, (message) => this.postMessage(message), this.clientFactory, this.beforeRequest);
    } finally {
      this.observer?.busyChanged(false);
    }
  }

  postMessage(message: unknown): void {
    void this.panel.webview.postMessage(message);
  }

  private html(webview: vscode.Webview): string {
    return renderChatHtml(webview, "panel");
    const nonce = getNonce();
    return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}';">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>OpenMindAI</title>
  <style>
    body {
      margin: 0;
      padding: 0;
      color: var(--vscode-foreground);
      background: var(--vscode-editor-background);
      font-family: var(--vscode-font-family);
    }
    .wrap {
      display: flex;
      flex-direction: column;
      height: 100vh;
    }
    .messages {
      flex: 1;
      overflow: auto;
      padding: 16px;
    }
    .message {
      border: 1px solid var(--vscode-panel-border);
      border-radius: 6px;
      margin-bottom: 12px;
      padding: 10px 12px;
      white-space: pre-wrap;
      line-height: 1.45;
    }
    .role {
      color: var(--vscode-descriptionForeground);
      font-size: 11px;
      font-weight: 600;
      margin-bottom: 6px;
      text-transform: uppercase;
    }
    .activity {
      display: none;
      align-items: center;
      gap: 10px;
      margin: 0 16px 10px;
      padding: 9px 10px;
      border: 1px solid var(--vscode-panel-border);
      border-radius: 6px;
      background: var(--vscode-sideBar-background);
    }
    .activity.visible {
      display: flex;
    }
    .activity-dot {
      width: 8px;
      height: 8px;
      border-radius: 999px;
      background: var(--vscode-button-background);
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--vscode-button-background) 45%, transparent);
      animation: activityPulse 1.1s ease-out infinite;
      flex: 0 0 auto;
    }
    .activity.done .activity-dot {
      animation: none;
      background: var(--vscode-testing-iconPassed, #4caf50);
    }
    .activity.error .activity-dot {
      animation: none;
      background: var(--vscode-testing-iconFailed, #f14c4c);
    }
    .activity-text {
      min-width: 0;
    }
    .activity-phase {
      color: var(--vscode-descriptionForeground);
      font-size: 11px;
      font-weight: 700;
      text-transform: uppercase;
    }
    .activity-detail {
      overflow: hidden;
      color: var(--vscode-foreground);
      font-size: 12px;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    @keyframes activityPulse {
      0% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--vscode-button-background) 45%, transparent); transform: scale(1); }
      70% { box-shadow: 0 0 0 8px transparent; transform: scale(0.92); }
      100% { box-shadow: 0 0 0 0 transparent; transform: scale(1); }
    }
    form {
      display: flex;
      gap: 8px;
      padding: 12px;
      border-top: 1px solid var(--vscode-panel-border);
      background: var(--vscode-sideBar-background);
    }
    textarea {
      flex: 1;
      min-height: 52px;
      max-height: 160px;
      resize: vertical;
      color: var(--vscode-input-foreground);
      background: var(--vscode-input-background);
      border: 1px solid var(--vscode-input-border);
      border-radius: 4px;
      padding: 8px;
      font-family: var(--vscode-font-family);
    }
    button {
      min-width: 72px;
      color: var(--vscode-button-foreground);
      background: var(--vscode-button-background);
      border: 0;
      border-radius: 4px;
      padding: 0 12px;
    }
    button:disabled {
      opacity: 0.6;
    }
  </style>
</head>
<body>
  <div class="wrap">
    <main class="messages" id="messages">
      <section class="message">
        <div class="role">OpenMindAI</div>
        Ask anything about your project. I can edit files directly in this workspace.
      </section>
    </main>
    <section class="activity" id="activity" aria-live="polite">
      <span class="activity-dot"></span>
      <span class="activity-text">
        <span class="activity-phase" id="activityPhase">Thinking</span>
        <span class="activity-detail" id="activityDetail"></span>
      </span>
    </section>
    <form id="form">
      <textarea id="input" placeholder="Ask OpenMindAI..."></textarea>
      <button id="send" type="submit">Send</button>
    </form>
  </div>
  <script nonce="${nonce}">
    const vscode = acquireVsCodeApi();
    const form = document.getElementById('form');
    const input = document.getElementById('input');
    const send = document.getElementById('send');
    const messages = document.getElementById('messages');
    const activity = document.getElementById('activity');
    const activityPhase = document.getElementById('activityPhase');
    const activityDetail = document.getElementById('activityDetail');
    let activityTimer = undefined;

    function append(role, content) {
      const section = document.createElement('section');
      section.className = 'message';
      const label = document.createElement('div');
      label.className = 'role';
      label.textContent = role;
      const body = document.createElement('div');
      body.textContent = content;
      section.append(label, body);
      messages.append(section);
      messages.scrollTop = messages.scrollHeight;
    }

    function setActivity(phase, detail) {
      clearTimeout(activityTimer);
      activity.className = 'activity visible ' + String(phase || '').toLowerCase();
      activityPhase.textContent = phase || 'Working';
      activityDetail.textContent = detail || '';
      if (phase === 'Done') {
        activityTimer = setTimeout(() => activity.classList.remove('visible'), 1400);
      }
    }

    form.addEventListener('submit', (event) => {
      event.preventDefault();
      const text = input.value.trim();
      if (!text) return;
      input.value = '';
      vscode.postMessage({ type: 'ask', text });
    });

    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
        form.requestSubmit();
      }
    });

    window.addEventListener('message', (event) => {
      const message = event.data;
      if (message.type === 'append') append(message.role, message.content);
      if (message.type === 'activity') setActivity(message.phase, message.detail);
      if (message.type === 'busy') {
        send.disabled = Boolean(message.busy);
        send.textContent = message.busy ? 'Thinking' : 'Send';
      }
    });
  </script>
</body>
</html>`;
  }
}

export class OpenMindChatViewProvider implements vscode.WebviewViewProvider {
  static readonly viewType = "openmindai.chatView";

  private view: vscode.WebviewView | undefined;
  private readonly messages: ChatMessage[] = [{ role: "system", content: SYSTEM_PROMPT }];
  observer: ChatSurfaceObserver | undefined;

  constructor(
    private readonly context: vscode.ExtensionContext,
    private readonly clientFactory: () => OpenMindClient,
    private readonly beforeRequest?: BeforeRequest
  ) {}

  get resolved(): boolean {
    return this.view !== undefined;
  }

  resolveWebviewView(view: vscode.WebviewView): void {
    this.view = view;
    view.webview.options = {
      enableScripts: true,
      localResourceRoots: [this.context.extensionUri]
    };
    view.webview.html = chatHtml(view.webview);
    view.webview.onDidReceiveMessage(async (message: { type?: string; text?: string }) => {
      if (this.observer?.receive(message)) return;
      if (message.type === "ask" && message.text?.trim()) {
        await this.ask(message.text.trim());
      }
    });
  }

  private async ask(text: string): Promise<void> {
    this.observer?.busyChanged(true);
    try {
      await runChatTurn(text, this.messages, (message) => this.postMessage(message), this.clientFactory, this.beforeRequest);
    } finally {
      this.observer?.busyChanged(false);
    }
  }

  postMessage(message: unknown): void {
    void this.view?.webview.postMessage(message);
  }
}

function chatHtml(webview: vscode.Webview): string {
  return renderChatHtml(webview, "view");
  const nonce = getNonce();
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}';">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>OpenMindAI</title>
  <style>
    body {
      margin: 0;
      padding: 0;
      color: var(--vscode-foreground);
      background: var(--vscode-editor-background);
      font-family: var(--vscode-font-family);
    }
    .wrap {
      display: flex;
      flex-direction: column;
      height: 100vh;
    }
    .messages {
      flex: 1;
      overflow: auto;
      padding: 12px;
    }
    .message {
      border: 1px solid var(--vscode-panel-border);
      border-radius: 6px;
      margin-bottom: 10px;
      padding: 9px 10px;
      white-space: pre-wrap;
      line-height: 1.45;
    }
    .role {
      color: var(--vscode-descriptionForeground);
      font-size: 11px;
      font-weight: 600;
      margin-bottom: 6px;
      text-transform: uppercase;
    }
    .activity {
      display: none;
      align-items: center;
      gap: 10px;
      margin: 0 10px 10px;
      padding: 9px 10px;
      border: 1px solid var(--vscode-panel-border);
      border-radius: 6px;
      background: var(--vscode-sideBar-background);
    }
    .activity.visible {
      display: flex;
    }
    .activity-dot {
      width: 8px;
      height: 8px;
      border-radius: 999px;
      background: var(--vscode-button-background);
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--vscode-button-background) 45%, transparent);
      animation: activityPulse 1.1s ease-out infinite;
      flex: 0 0 auto;
    }
    .activity.done .activity-dot {
      animation: none;
      background: var(--vscode-testing-iconPassed, #4caf50);
    }
    .activity.error .activity-dot {
      animation: none;
      background: var(--vscode-testing-iconFailed, #f14c4c);
    }
    .activity-text {
      min-width: 0;
    }
    .activity-phase {
      color: var(--vscode-descriptionForeground);
      font-size: 11px;
      font-weight: 700;
      text-transform: uppercase;
    }
    .activity-detail {
      overflow: hidden;
      color: var(--vscode-foreground);
      font-size: 12px;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    @keyframes activityPulse {
      0% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--vscode-button-background) 45%, transparent); transform: scale(1); }
      70% { box-shadow: 0 0 0 8px transparent; transform: scale(0.92); }
      100% { box-shadow: 0 0 0 0 transparent; transform: scale(1); }
    }
    form {
      display: flex;
      flex-direction: column;
      gap: 8px;
      padding: 10px;
      border-top: 1px solid var(--vscode-panel-border);
      background: var(--vscode-sideBar-background);
    }
    textarea {
      min-height: 58px;
      max-height: 180px;
      resize: vertical;
      color: var(--vscode-input-foreground);
      background: var(--vscode-input-background);
      border: 1px solid var(--vscode-input-border);
      border-radius: 4px;
      padding: 8px;
      font-family: var(--vscode-font-family);
    }
    button {
      height: 30px;
      color: var(--vscode-button-foreground);
      background: var(--vscode-button-background);
      border: 0;
      border-radius: 4px;
      padding: 0 12px;
    }
    button:disabled {
      opacity: 0.6;
    }
  </style>
</head>
<body>
  <div class="wrap">
    <main class="messages" id="messages">
      <section class="message">
        <div class="role">OpenMindAI</div>
        Ask anything about your project. I can edit files directly in this workspace.
      </section>
    </main>
    <section class="activity" id="activity" aria-live="polite">
      <span class="activity-dot"></span>
      <span class="activity-text">
        <span class="activity-phase" id="activityPhase">Thinking</span>
        <span class="activity-detail" id="activityDetail"></span>
      </span>
    </section>
    <form id="form">
      <textarea id="input" placeholder="Ask OpenMindAI..."></textarea>
      <button id="send" type="submit">Send</button>
    </form>
  </div>
  <script nonce="${nonce}">
    const vscode = acquireVsCodeApi();
    const form = document.getElementById('form');
    const input = document.getElementById('input');
    const send = document.getElementById('send');
    const messages = document.getElementById('messages');
    const activity = document.getElementById('activity');
    const activityPhase = document.getElementById('activityPhase');
    const activityDetail = document.getElementById('activityDetail');
    let activityTimer = undefined;

    function append(role, content) {
      const section = document.createElement('section');
      section.className = 'message';
      const label = document.createElement('div');
      label.className = 'role';
      label.textContent = role;
      const body = document.createElement('div');
      body.textContent = content;
      section.append(label, body);
      messages.append(section);
      messages.scrollTop = messages.scrollHeight;
    }

    function setActivity(phase, detail) {
      clearTimeout(activityTimer);
      activity.className = 'activity visible ' + String(phase || '').toLowerCase();
      activityPhase.textContent = phase || 'Working';
      activityDetail.textContent = detail || '';
      if (phase === 'Done') {
        activityTimer = setTimeout(() => activity.classList.remove('visible'), 1400);
      }
    }

    form.addEventListener('submit', (event) => {
      event.preventDefault();
      const text = input.value.trim();
      if (!text) return;
      input.value = '';
      vscode.postMessage({ type: 'ask', text });
    });

    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
        form.requestSubmit();
      }
    });

    window.addEventListener('message', (event) => {
      const message = event.data;
      if (message.type === 'append') append(message.role, message.content);
      if (message.type === 'activity') setActivity(message.phase, message.detail);
      if (message.type === 'busy') {
        send.disabled = Boolean(message.busy);
        send.textContent = message.busy ? 'Thinking' : 'Send';
      }
    });
  </script>
</body>
</html>`;
}

export async function buildWorkspaceRequest(userText: string, report?: ActivityReporter): Promise<string> {
  const workspace = vscode.workspace.workspaceFolders?.[0];
  const editor = vscode.window.activeTextEditor;
  const parts = [`User request:\n${userText}`];

  if (workspace) {
    parts.push(`Workspace root: ${workspace.uri.fsPath}`);
    report?.("Reading", "workspace file tree");
    const files = await vscode.workspace.findFiles(
      "**/*",
      "{**/node_modules/**,**/.git/**,**/dist/**,**/target/**,**/build/**}",
      30
    );
    parts.push(`Project files:\n${files.map((uri) => vscode.workspace.asRelativePath(uri)).join("\n")}`);
  } else {
    parts.push("No workspace folder is open.");
  }

  if (editor) {
    const document = editor.document;
    report?.("Reading", vscode.workspace.asRelativePath(document.uri));
    const selected = editor.selection.isEmpty ? "" : document.getText(editor.selection);
    const text = selected || document.getText();
    const maxChars = vscode.workspace.getConfiguration("openmindai").get<number>("maxContextCharacters", 800);
    const preview = text.length > maxChars ? `${text.slice(0, maxChars)}\n[truncated]` : text;
    parts.push(
      [
        `Active file: ${vscode.workspace.asRelativePath(document.uri)}`,
        `Language: ${document.languageId}`,
        selected ? "Selected text:" : "File content:",
        `\`\`\`${document.languageId}`,
        preview,
        "```"
      ].join("\n")
    );
  } else {
    parts.push("No active editor.");
  }

  parts.push(
    [
      "If this request needs file changes, reply with only this block and no other text:",
      "```" + EDIT_FENCE_TAG,
      EDIT_PLAN_EXAMPLE,
      "```",
      "Otherwise answer normally without an edit block."
    ].join("\n")
  );

  return parts.join("\n\n");
}

/**
 * Shows the visible part of an answer and applies any OpenMindAI edit plan it carries.
 * Raw edit JSON is never shown as chat text. Returns the workspace-relative paths written.
 */
export async function deliverAnswer(
  answer: string,
  report: ActivityReporter | undefined,
  show: (text: string) => void,
  userText?: string
): Promise<string[]> {
  // Files are only written when the user's message asked for a change.
  const editRequested = userText === undefined || looksLikeEditRequest(userText);
  let extraction;
  try {
    extraction = extractEditPlan(answer);
  } catch (error) {
    const visible = stripEditFences(answer);
    if (visible) show(visible);
    if (!editRequested) return [];
    throw error;
  }
  if (extraction.kind === "none") {
    const text = stripEditFences(answer);
    if (text) show(text);
    return [];
  }
  if (!editRequested) {
    const text = extraction.visibleText;
    show(text ? `${text}\n\n${UNREQUESTED_EDIT_NOTE}` : UNREQUESTED_EDIT_NOTE);
    return [];
  }
  const text = extraction.visibleText || extraction.plan.summary?.trim() || "";
  if (text) show(text);
  return applyEditPlan(extraction.plan, report);
}

/** Validates every target before writing anything, then applies the plan as one edit. */
export async function applyEditPlan(plan: EditPlan, report?: ActivityReporter): Promise<string[]> {
  const workspace = vscode.workspace.workspaceFolders?.[0];
  if (!workspace) {
    throw new Error("OpenMindAI returned edits, but no workspace folder is open.");
  }

  const rootPath = path.resolve(workspace.uri.fsPath);
  const targets = resolveEditTargets(rootPath, plan);
  for (const target of targets) {
    await assertInsideRealWorkspace(rootPath, target.absolute);
  }

  // Existing files are edited through the open document (undoable, keeps editor state).
  // New files are written directly: a createFile + insert WorkspaceEdit leaves the content
  // in an unsaved document, and saveAll does not reliably persist it.
  const edit = new vscode.WorkspaceEdit();
  const created: EditTarget[] = [];
  const edited: vscode.Uri[] = [];
  for (const target of targets) {
    const uri = vscode.Uri.file(target.absolute);
    if (await fileExists(uri)) {
      const document = await vscode.workspace.openTextDocument(uri);
      const fullRange = new vscode.Range(
        document.positionAt(0),
        document.positionAt(document.getText().length)
      );
      edit.replace(uri, fullRange, target.content);
      edited.push(uri);
    } else {
      created.push(target);
    }
  }

  for (const target of created) {
    report?.("Writing", target.relative);
    const uri = vscode.Uri.file(target.absolute);
    await vscode.workspace.fs.createDirectory(parentUri(uri));
    await vscode.workspace.fs.writeFile(uri, Buffer.from(target.content, "utf8"));
  }
  if (edited.length) {
    for (const target of targets) {
      if (!created.includes(target)) report?.("Writing", target.relative);
    }
    const ok = await vscode.workspace.applyEdit(edit);
    if (!ok) throw new Error("VS Code could not apply the OpenMindAI edit plan.");
    for (const uri of edited) {
      const document = await vscode.workspace.openTextDocument(uri);
      if (document.isDirty && !(await document.save())) {
        throw new Error(`VS Code could not save ${vscode.workspace.asRelativePath(uri)}.`);
      }
    }
  }
  return targets.map((target) => target.relative);
}

async function fileExists(uri: vscode.Uri): Promise<boolean> {
  try {
    await vscode.workspace.fs.stat(uri);
    return true;
  } catch {
    return false;
  }
}

function parentUri(uri: vscode.Uri): vscode.Uri {
  return vscode.Uri.file(path.dirname(uri.fsPath));
}

function renderChatHtml(webview: vscode.Webview, variant: "panel" | "view"): string {
  const nonce = getNonce();
  const compactClass = variant === "view" ? "compact" : "";
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}';">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>OpenMindAI</title>
  <style>
    :root {
      color-scheme: dark;
      --om-bg: var(--vscode-editor-background);
      --om-panel: color-mix(in srgb, var(--vscode-sideBar-background) 88%, var(--vscode-editor-background));
      --om-panel-2: color-mix(in srgb, var(--vscode-editorWidget-background) 80%, var(--vscode-sideBar-background));
      --om-border: var(--vscode-panel-border);
      --om-text: var(--vscode-foreground);
      --om-muted: var(--vscode-descriptionForeground);
      --om-accent: var(--vscode-button-background);
      --om-accent-text: var(--vscode-button-foreground);
      --om-success: var(--vscode-testing-iconPassed, #3fb950);
      --om-danger: var(--vscode-testing-iconFailed, #f85149);
    }

    * { box-sizing: border-box; }

    body {
      margin: 0;
      color: var(--om-text);
      background: var(--om-bg);
      font-family: var(--vscode-font-family);
      font-size: var(--vscode-font-size);
    }

    button,
    textarea {
      font: inherit;
    }

    button {
      border: 0;
      cursor: pointer;
    }

    .wrap {
      display: grid;
      grid-template-rows: auto 1fr auto;
      gap: 8px;
      height: 100vh;
      padding: 8px;
      background:
        linear-gradient(180deg, rgba(255,255,255,0.018), transparent 34%),
        var(--om-bg);
    }

    .agent-top {
      display: grid;
      gap: 8px;
      padding: 10px;
      border: 1px solid var(--om-border);
      border-radius: 8px;
      background: var(--om-panel);
    }

    .agent-title {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
    }

    .agent-title-main {
      min-width: 0;
    }

    .eyebrow {
      display: block;
      color: var(--om-muted);
      font-size: 10px;
      font-weight: 700;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }

    .agent-title strong {
      display: block;
      margin-top: 2px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-size: 13px;
    }

    .status-pill {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      min-height: 24px;
      padding: 0 8px;
      border: 1px solid var(--om-border);
      border-radius: 999px;
      color: var(--om-muted);
      background: color-mix(in srgb, var(--om-panel-2) 82%, transparent);
      font-size: 11px;
      white-space: nowrap;
    }

    .status-dot {
      width: 7px;
      height: 7px;
      border-radius: 999px;
      background: var(--om-success);
      box-shadow: 0 0 0 3px color-mix(in srgb, var(--om-success) 16%, transparent);
    }

    .quick-row {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 6px;
    }

    .quick-row button {
      min-width: 0;
      min-height: 30px;
      padding: 0 8px;
      border: 1px solid var(--om-border);
      border-radius: 6px;
      background: transparent;
      color: var(--om-muted);
      text-align: left;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .quick-row button:hover {
      background: var(--om-panel-2);
      color: var(--om-text);
    }

    .timeline {
      min-height: 0;
      overflow: auto;
      padding: 2px 2px 8px;
    }

    .message {
      position: relative;
      display: grid;
      grid-template-columns: 18px minmax(0, 1fr);
      gap: 9px;
      margin: 0 0 10px;
    }

    .message::before {
      content: "";
      position: absolute;
      left: 8px;
      top: 22px;
      bottom: -12px;
      width: 1px;
      background: var(--om-border);
    }

    .message:last-child::before {
      display: none;
    }

    .node {
      position: relative;
      z-index: 1;
      width: 9px;
      height: 9px;
      margin: 12px auto 0;
      border: 2px solid var(--om-bg);
      border-radius: 999px;
      background: var(--om-muted);
      box-shadow: 0 0 0 1px var(--om-border);
    }

    .message.user .node {
      background: var(--om-accent);
    }

    .message.assistant .node,
    .message.openmindai .node {
      background: var(--om-success);
    }

    .message.error .node {
      background: var(--om-danger);
    }

    .bubble {
      min-width: 0;
      padding: 10px 11px;
      border: 1px solid var(--om-border);
      border-radius: 8px;
      background: color-mix(in srgb, var(--om-panel) 82%, transparent);
      line-height: 1.5;
      white-space: pre-wrap;
    }

    .message.user .bubble {
      background: color-mix(in srgb, var(--om-accent) 16%, var(--om-panel));
    }

    .role {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      margin-bottom: 6px;
      color: var(--om-muted);
      font-size: 10px;
      font-weight: 700;
      letter-spacing: 0.06em;
      text-transform: uppercase;
    }

    .body {
      min-width: 0;
      overflow-wrap: anywhere;
    }

    .activity {
      display: none;
      grid-template-columns: 18px minmax(0, 1fr);
      gap: 9px;
      margin-bottom: 10px;
    }

    .activity.visible {
      display: grid;
    }

    .activity-dot {
      width: 9px;
      height: 9px;
      margin: 12px auto 0;
      border-radius: 999px;
      background: var(--om-accent);
      animation: activityPulse 1.1s ease-out infinite;
    }

    .activity.done .activity-dot {
      animation: none;
      background: var(--om-success);
    }

    .activity.error .activity-dot {
      animation: none;
      background: var(--om-danger);
    }

    .activity-card {
      display: grid;
      gap: 3px;
      padding: 9px 10px;
      border: 1px solid var(--om-border);
      border-radius: 8px;
      background: var(--om-panel);
    }

    .activity-phase {
      color: var(--om-muted);
      font-size: 10px;
      font-weight: 800;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }

    .activity-detail {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-size: 12px;
    }

    @keyframes activityPulse {
      0% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--om-accent) 45%, transparent); transform: scale(1); }
      70% { box-shadow: 0 0 0 7px transparent; transform: scale(0.9); }
      100% { box-shadow: 0 0 0 0 transparent; transform: scale(1); }
    }

    .composer {
      display: grid;
      gap: 8px;
      padding: 8px;
      border: 1px solid var(--om-border);
      border-radius: 8px;
      background: var(--om-panel);
    }

    textarea {
      width: 100%;
      min-height: 72px;
      max-height: 190px;
      resize: vertical;
      padding: 9px 10px;
      border: 1px solid var(--vscode-input-border, var(--om-border));
      border-radius: 7px;
      outline: none;
      color: var(--vscode-input-foreground);
      background: var(--vscode-input-background);
      font-family: var(--vscode-font-family);
      line-height: 1.45;
    }

    textarea:focus {
      border-color: var(--vscode-focusBorder, var(--om-accent));
    }

    .composer-footer {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
    }

    .hint {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      color: var(--om-muted);
      font-size: 11px;
    }

    #send {
      min-width: 74px;
      min-height: 30px;
      padding: 0 12px;
      border-radius: 6px;
      background: var(--om-accent);
      color: var(--om-accent-text);
      font-weight: 700;
    }

    #send:disabled {
      opacity: 0.65;
      cursor: default;
    }

    .compact .wrap {
      padding: 6px;
      gap: 6px;
    }

    .compact .agent-top {
      padding: 8px;
    }

    .compact .quick-row {
      grid-template-columns: 1fr;
    }

    .compact .bubble {
      padding: 9px;
    }
  </style>
</head>
<body class="${compactClass}">
  <div class="wrap">
    <header class="agent-top">
      <div class="agent-title">
        <div class="agent-title-main">
          <span class="eyebrow">OpenMindAI Agent</span>
          <strong>Workspace coding assistant</strong>
        </div>
        <span class="status-pill"><span class="status-dot"></span> Local</span>
      </div>
      <div class="quick-row" aria-label="Suggested actions">
        <button type="button" data-template="Explain the active file">Explain file</button>
        <button type="button" data-template="Audit this workspace and report risks">Audit workspace</button>
        <button type="button" data-template="Fix the selected code">Fix selection</button>
      </div>
    </header>

    <main class="timeline" id="messages">
      <section class="message assistant">
        <span class="node"></span>
        <div class="bubble">
          <div class="role">OpenMindAI</div>
          <div class="body">Ask about this workspace, request edits, or run a focused review. I can read the active file context and apply safe file changes when you approve the request.</div>
        </div>
      </section>
      <section class="activity" id="activity" aria-live="polite">
        <span class="activity-dot"></span>
        <span class="activity-card">
          <span class="activity-phase" id="activityPhase">Thinking</span>
          <span class="activity-detail" id="activityDetail"></span>
        </span>
      </section>
    </main>

    <form class="composer" id="form">
      <textarea id="input" placeholder="Ask OpenMindAI to inspect, explain, edit, or review this workspace..."></textarea>
      <div class="composer-footer">
        <span class="hint">Ctrl/⌘ + Enter to send</span>
        <button id="send" type="submit">Send</button>
      </div>
    </form>
  </div>
  <script nonce="${nonce}">
    const vscode = acquireVsCodeApi();
    const form = document.getElementById('form');
    const input = document.getElementById('input');
    const send = document.getElementById('send');
    const messages = document.getElementById('messages');
    const activity = document.getElementById('activity');
    const activityPhase = document.getElementById('activityPhase');
    const activityDetail = document.getElementById('activityDetail');
    let activityTimer = undefined;

    function roleClass(role) {
      const normalized = String(role || '').toLowerCase();
      if (normalized.includes('user')) return 'user';
      if (normalized.includes('error')) return 'error';
      if (normalized.includes('openmind')) return 'openmindai';
      return 'assistant';
    }

    function append(role, content) {
      const section = document.createElement('section');
      section.className = 'message ' + roleClass(role);
      const node = document.createElement('span');
      node.className = 'node';
      const bubble = document.createElement('div');
      bubble.className = 'bubble';
      const label = document.createElement('div');
      label.className = 'role';
      label.textContent = role;
      const body = document.createElement('div');
      body.className = 'body';
      body.textContent = content || '';
      bubble.append(label, body);
      section.append(node, bubble);
      messages.insertBefore(section, activity);
      messages.scrollTop = messages.scrollHeight;
    }

    function setActivity(phase, detail) {
      clearTimeout(activityTimer);
      activity.className = 'activity visible ' + String(phase || '').toLowerCase();
      activityPhase.textContent = phase || 'Working';
      activityDetail.textContent = detail || '';
      messages.scrollTop = messages.scrollHeight;
      if (phase === 'Done') {
        activityTimer = setTimeout(() => activity.classList.remove('visible'), 1400);
      }
    }

    form.addEventListener('submit', (event) => {
      event.preventDefault();
      const text = input.value.trim();
      if (!text) return;
      input.value = '';
      vscode.postMessage({ type: 'ask', text });
    });

    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
        form.requestSubmit();
      }
    });

    document.querySelectorAll('[data-template]').forEach((button) => {
      button.addEventListener('click', () => {
        input.value = button.getAttribute('data-template') || '';
        input.focus();
      });
    });

    window.addEventListener('message', (event) => {
      const message = event.data;
      if (message.type === 'append') append(message.role, message.content);
      if (message.type === 'activity') setActivity(message.phase, message.detail);
      if (message.type === 'busy') {
        send.disabled = Boolean(message.busy);
        send.textContent = message.busy ? 'Working' : 'Send';
      }
    });
${extraWebviewScript}
  </script>
</body>
</html>`;
}

function getNonce(): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let value = "";
  for (let i = 0; i < 32; i += 1) {
    value += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return value;
}
