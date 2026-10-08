// Real VS Code UI integration suite. Loaded by VS Code via --extensionTestsPath; runs inside
// the Extension Development Host against the running OpenMindAI desktop app.
const assert = require("node:assert/strict");
const cp = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const vscode = require("vscode");

const workspace = process.env.OPENMINDAI_E2E_WORKSPACE;
const reportPath = process.env.OPENMINDAI_E2E_REPORT;
const REQUEST_TIMEOUT_MS = 8 * 60_000;
const ASK_PROMPT = "Explain ownership in Rust in three sentences.";
const CHAT_PROMPT = "Explain what a TypeScript generic is in two sentences.";
const PARTICIPANT_PROMPT = "Explain the difference between a Rust enum and struct in three sentences.";
const TEST_MODEL_MARKER = "[openmindai-e2e-test-model]";

const results = [];
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const read = (relative) => fs.readFileSync(path.join(workspace, relative), "utf8");
const snapshotFiles = () =>
  Object.fromEntries(
    walk(workspace).map((file) => [path.relative(workspace, file).replace(/\\/g, "/"), fs.readFileSync(file, "utf8")])
  );

function walk(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.name === ".vscode") return [];
    return entry.isDirectory() ? walk(full) : [full];
  });
}

async function step(name, fn) {
  lastTurn = undefined;
  const started = Date.now();
  try {
    const detail = await fn();
    results.push({ name, ok: true, seconds: Math.round((Date.now() - started) / 1000), detail });
  } catch (error) {
    results.push({
      name,
      ok: false,
      seconds: Math.round((Date.now() - started) / 1000),
      error: error instanceof Error ? error.message : String(error),
      lastWebviewTurn: lastTurn
    });
  }
  // Written after every step so a crashed window still leaves the results so far.
  fs.writeFileSync(reportPath, JSON.stringify({ results, complete: false }, null, 2));
}

async function waitFor(predicate, timeoutMs, label) {
  const started = Date.now();
  for (;;) {
    const value = await predicate();
    if (value) return value;
    if (Date.now() - started > timeoutMs) throw new Error(`timed out waiting for ${label}`);
    await sleep(500);
  }
}

/** Types into the real webview form and waits for the turn to finish rendering. */
/** Snapshot that gives up after `ms` (a webview that is still loading drops messages). */
function snapshotWithin(handle, ms) {
  return Promise.race([handle.snapshot(), sleep(ms).then(() => undefined)]);
}

/** Waits until the webview page is loaded and answering, re-revealing it if needed. */
async function untilResponsive(handle, reveal) {
  const started = Date.now();
  for (;;) {
    const snapshot = await snapshotWithin(handle, 2000);
    if (snapshot) return snapshot;
    if (Date.now() - started > 60_000) throw new Error("webview did not become responsive");
    await reveal?.();
    await sleep(1000);
  }
}

async function chatThroughWebview(handle, text, reveal) {
  const before = (await untilResponsive(handle, reveal)).bubbles.length;
  handle.submit(text);
  // A turn that fails fast (for example while Coding Workspace is disabled) can start and end
  // between two polls, so new bubbles also count as "started".
  await waitFor(
    async () => handle.busy || ((await snapshotWithin(handle, 2000))?.bubbles.length ?? 0) > before,
    30_000,
    "webview request to start"
  );
  const progress = new Set();
  await waitFor(async () => {
    const snapshot = await handle.snapshot();
    if (snapshot.activity.detail) progress.add(`${snapshot.activity.phase}: ${snapshot.activity.detail}`);
    return !handle.busy;
  }, REQUEST_TIMEOUT_MS, "webview response");
  await sleep(300);
  const snapshot = await handle.snapshot();
  const turn = { bubbles: snapshot.bubbles.slice(before), activity: snapshot.activity, progress: [...progress] };
  lastTurn = turn;
  return turn;
}

let lastTurn;

/** A broken runtime repeats one short fragment until the token limit. */
function assertNotDegenerate(text, label) {
  assert.doesNotMatch(text, /([\s\S]{1,32}?)\1{19,}/, `${label} is degenerate repetition: ${text.slice(0, 120)}`);
  assert.doesNotMatch(text, /<tool_call>/, `${label} contains raw tool-call markup`);
}

function assertCleanBubbles(bubbles) {
  for (const bubble of bubbles) {
    assert.ok(bubble.text.trim(), `empty ${bubble.role} bubble`);
    assert.doesNotMatch(bubble.text, /"files"\s*:/, "raw edit JSON shown in chat");
    assert.doesNotMatch(bubble.text, /<think>|<\/think>/, "reasoning tags shown in chat");
    assert.doesNotMatch(bubble.text, /OpenMindAI error/, `error bubble: ${bubble.text}`);
    assertNotDegenerate(bubble.text, `${bubble.role} bubble`);
  }
}

async function openAndSelect(relative, startLine, endLine) {
  const document = await vscode.workspace.openTextDocument(path.join(workspace, relative));
  const editor = await vscode.window.showTextDocument(document, vscode.ViewColumn.One);
  editor.selection = new vscode.Selection(startLine, 0, endLine, document.lineAt(endLine).text.length);
  return editor;
}

function setCodingEnabled(enabled) {
  const sqlite = process.env.OPENMINDAI_SQLITE;
  const db = process.env.OPENMINDAI_DB;
  const query = (sql) =>
    cp.execFileSync(sqlite, [db, ".timeout 5000", sql], { encoding: "utf8" }).trim();
  const original = query("SELECT value_json FROM app_settings WHERE key = 'app.preferences';");
  const preferences = JSON.parse(original);
  preferences.codingEnabled = enabled;
  const escaped = JSON.stringify(preferences).replace(/'/g, "''");
  query(`UPDATE app_settings SET value_json = '${escaped}' WHERE key = 'app.preferences';`);
  return () => {
    const restore = original.replace(/'/g, "''");
    query(`UPDATE app_settings SET value_json = '${restore}' WHERE key = 'app.preferences';`);
    return JSON.parse(query("SELECT value_json FROM app_settings WHERE key = 'app.preferences';"));
  };
}

exports.run = async function run() {
  const extension = vscode.extensions.getExtension("openmindai.openmindai-vscode");
  assert.ok(extension, "extension not loaded");
  const api = await extension.activate();
  assert.ok(api, "test API missing; extension must run in Test mode");

  await step("connection: discovers coding-agent-endpoint.json and the Agent Setup model", async () => {
    await vscode.commands.executeCommand("openmindai.checkConnection");
    const target = api.target();
    assert.equal(target?.kind, "coding-agent");
    assert.ok(target.token, "session token not read from descriptor");
    const model = target.agent?.model;
    assert.match(model?.repository ?? "", /Nemotron/i);
    return {
      endpoint: target.endpoint,
      model: `${model.name} (${model.displayName ?? model.repository})`,
      runtimeState: target.agent?.runtime?.state,
      placement: target.agent?.runtime?.runtimePlacement,
      log: api.testLog().filter((entry) => entry.kind === "connection")
    };
  });

  await step("ask command: Rust ownership", async () => {
    api.clearTestLog();
    const started = Date.now();
    await vscode.commands.executeCommand("openmindai.ask", ASK_PROMPT);
    const log = api.testLog();
    const error = log.find((entry) => entry.kind === "error");
    assert.ok(!error, error?.text);
    const markdown = log.find((entry) => entry.kind === "markdown");
    assert.ok(markdown, "no answer document");
    const answer = markdown.text.split("\n").slice(1).join("\n").trim();
    assert.ok(answer.length > 40, "answer too short");
    assert.doesNotMatch(answer, /<think>/);
    assertNotDegenerate(answer, "Ask answer");
    const shown = vscode.workspace.textDocuments.some((doc) => doc.getText().startsWith("# OpenMindAI Answer"));
    assert.ok(shown, "answer document not opened in the editor");
    return {
      seconds: Math.round((Date.now() - started) / 1000),
      loadingProgress: log.filter((entry) => entry.kind === "progress").map((entry) => entry.text),
      answerExcerpt: answer.slice(0, 300)
    };
  });

  await step("sidebar: normal coding question through the real webview form", async () => {
    await vscode.commands.executeCommand("openmindai.chatView.focus");
    await waitFor(() => api.viewProvider.resolved, 30_000, "sidebar webview");
    await sleep(1500);
    const before = snapshotFiles();
    const turn = await chatThroughWebview(
      api.viewProvider.test,
      "What is the difference between let and const in TypeScript? Answer in two sentences."
    );
    assertCleanBubbles(turn.bubbles);
    const answer = turn.bubbles.find((bubble) => bubble.role === "assistant");
    assert.ok(answer && answer.text.length > 30, "no answer bubble");
    assert.deepEqual(snapshotFiles(), before, "a normal question changed files");
    return { bubbles: turn.bubbles, progress: turn.progress };
  });

  await step("sidebar: existing-file edit through the real webview form", async () => {
    await openAndSelect("src/greet.ts", 0, 0);
    await vscode.commands.executeCommand("openmindai.chatView.focus");
    await sleep(1500);
    const before = snapshotFiles();
    const turn = await chatThroughWebview(
      api.viewProvider.test,
      "Change greet in src/greet.ts so it returns `Hello, ${name}!` using a template literal. Do not change any other file."
    );
    assertCleanBubbles(turn.bubbles);
    const after = snapshotFiles();
    const changed = Object.keys(after).filter((file) => after[file] !== before[file]);
    assert.deepEqual(changed, ["src/greet.ts"]);
    assert.match(after["src/greet.ts"], /`Hello, \$\{name\}!`/);
    assert.ok(turn.bubbles.some((bubble) => /Applied edits/.test(bubble.text)), "no applied-edits bubble");
    return { bubbles: turn.bubbles, progress: turn.progress, greet: after["src/greet.ts"] };
  });

  await step("panel: create src/math.ts (multiply) through the real webview form", async () => {
    api.openPanel();
    const panel = await waitFor(() => api.activePanel(), 30_000, "editor panel");
    await sleep(1500);
    const before = snapshotFiles();
    const turn = await chatThroughWebview(
      panel.test,
      "Create src/math.ts with an exported multiply(a: number, b: number) function."
    );
    assertCleanBubbles(turn.bubbles);
    const after = snapshotFiles();
    const changed = Object.keys(after).filter((file) => after[file] !== before[file]);
    assert.deepEqual(changed, ["src/math.ts"]);
    assert.match(
      after["src/math.ts"],
      /export\s+(function\s+multiply\s*\(\s*a\s*:\s*number\s*,\s*b\s*:\s*number|const\s+multiply)/
    );
    assert.match(after["src/math.ts"], /a\s*\*\s*b/);
    return { bubbles: turn.bubbles, progress: turn.progress, math: after["src/math.ts"] };
  });

  await step("chat view: real @openmindai dispatch through the VS Code Chat input", async () => {
    const testModel = vscode.extensions.getExtension("openmindai-test.openmindai-e2e-test-model");
    assert.ok(testModel, "test-only chat model extension not loaded");
    await testModel.activate();
    const models = await vscode.lm.selectChatModels({ vendor: "openmindai-e2e" });
    assert.ok(models.length > 0, "test chat model not registered");

    const savedClipboard = await vscode.env.clipboard.readText();
    api.clearTestLog();
    let transcript = "";
    try {
      // VS Code attaches the picker's model to every participant request and refuses to
      // dispatch when none is selected, so pick the test model like a user would.
      await vscode.commands.executeCommand("workbench.action.chat.open", { query: "", isPartialQuery: true, mode: "ask" });
      await sleep(1500);
      await vscode.commands.executeCommand("workbench.action.chat.changeModel", {
        vendor: "openmindai-e2e",
        id: "openmindai-e2e-echo",
        family: "openmindai-e2e"
      });
      await sleep(500);
      // Same as typing into the Chat input and pressing Enter: the Chat widget's setInput
      // followed by acceptInput. VS Code itself must resolve @openmindai to the participant.
      await vscode.commands.executeCommand("workbench.action.chat.open", {
        query: `@openmindai ${CHAT_PROMPT}`,
        isPartialQuery: false,
        mode: "ask"
      });
      await waitFor(
        () => api.testLog().some((entry) => entry.kind === "participant"),
        60_000,
        "VS Code Chat to dispatch the typed message to @openmindai"
      );
      await waitFor(
        () =>
          api
            .testLog()
            .some((entry) => entry.kind === "participant-progress" && /^(Done|Error)/.test(entry.text)),
        REQUEST_TIMEOUT_MS,
        "participant response"
      );
      await sleep(2000);
      // Read what the Chat view rendered.
      await vscode.commands.executeCommand("workbench.action.chat.copyAll");
      transcript = await vscode.env.clipboard.readText();
    } finally {
      await vscode.env.clipboard.writeText(savedClipboard);
    }

    const log = api.testLog();
    const dispatched = log.find((entry) => entry.kind === "participant");
    assert.equal(dispatched.text.trim(), CHAT_PROMPT, "participant received a different prompt");
    const progress = log.filter((entry) => entry.kind === "participant-progress").map((entry) => entry.text);
    assert.ok(progress.includes("Done: Response ready"), `participant did not finish: ${progress.join(" | ")}`);
    const waiting = progress.find((text) => /Waiting for/.test(text));
    assert.match(waiting ?? "", /OpenMindAI coding agent: OpenAgent Lite \(NVIDIA Nemotron/);
    const answer = log
      .filter((entry) => entry.kind === "participant-markdown")
      .map((entry) => entry.text)
      .join("")
      .trim();
    assert.ok(answer.length > 40, "participant produced no answer");
    assert.doesNotMatch(answer, /OpenMindAI error|<think>|"files"\s*:/);
    assertNotDegenerate(answer, "Chat view answer");
    assertNotDegenerate(transcript, "Chat view transcript");

    // The answer must be rendered in the Chat view, and never by the test model.
    assert.ok(transcript.includes(answer.slice(0, 60)), "answer not rendered in the Chat view");
    assert.ok(!transcript.includes(TEST_MODEL_MARKER), "test model answered instead of OpenMindAI");
    assert.doesNotMatch(transcript, /<think>|reasoning_content/);
    const target = api.target();
    assert.equal(target?.kind, "coding-agent");
    assert.match(target.agent?.model?.repository ?? "", /Nemotron/);
    return {
      testModels: models.map((model) => model.name),
      progress,
      answerExcerpt: answer.slice(0, 300),
      chatViewTranscriptExcerpt: transcript.slice(0, 500)
    };
  });

  await step("chat participant handler (inside the real extension host)", async () => {
    const events = await api.runParticipant(PARTICIPANT_PROMPT);
    const markdown = events.filter((event) => event.kind === "markdown").map((event) => event.text).join("");
    assert.ok(markdown.trim().length > 40, "participant produced no answer");
    assert.doesNotMatch(markdown, /OpenMindAI error|<think>/);
    assertNotDegenerate(markdown, "participant answer");
    const waiting = events.find((event) => event.kind === "progress" && /Waiting for/.test(event.text));
    assert.match(waiting?.text ?? "", /OpenMindAI coding agent: OpenAgent/);
    return { waiting: waiting.text, answerExcerpt: markdown.slice(0, 300) };
  });

  await step("explain selection command", async () => {
    api.clearTestLog();
    await openAndSelect("src/shapes.rs", 5, 10);
    await vscode.commands.executeCommand("openmindai.explainSelection");
    const log = api.testLog();
    assert.ok(!log.some((entry) => entry.kind === "error"), JSON.stringify(log));
    const markdown = log.find((entry) => entry.kind === "markdown" && entry.text.startsWith("OpenMindAI Explanation"));
    assert.ok(markdown, "no explanation document");
    assertNotDegenerate(markdown.text, "explanation");
    return { excerpt: markdown.text.slice(0, 300) };
  });

  await step("fix selection command (diff preview, then apply)", async () => {
    api.clearTestLog();
    const before = snapshotFiles();
    const editor = await openAndSelect("src/shapes.rs", 5, 10);
    await vscode.commands.executeCommand("openmindai.fixSelection", { testAutoApply: true });
    const log = api.testLog();
    assert.ok(!log.some((entry) => entry.kind === "error"), JSON.stringify(log));
    assert.ok(log.some((entry) => entry.kind === "fix-preview"), "no diff preview");
    await editor.document.save();
    const after = snapshotFiles();
    const changed = Object.keys(after).filter((file) => after[file] !== before[file]);
    assert.deepEqual(changed, ["src/shapes.rs"]);
    assert.match(after["src/shapes.rs"], /pub enum Shape/, "code outside the selection changed");
    return { shapes: after["src/shapes.rs"] };
  });

  if (process.env.OPENMINDAI_SQLITE && process.env.OPENMINDAI_DB) {
    await step("Coding Workspace disabled: clear message, no fallback to Core", async () => {
      api.clearTestLog();
      const restore = setCodingEnabled(false);
      let restored;
      try {
        await vscode.commands.executeCommand("openmindai.ask", ASK_PROMPT);
        // The Chat view shares the secondary sidebar and can recreate the OpenMindAI view, so
        // wait until the view's page answers again before typing into it.
        const sidebarTurn = await chatThroughWebview(api.viewProvider.test, "Say hello.", () =>
          vscode.commands.executeCommand("openmindai.chatView.focus")
        );
        const error = api.testLog().find((entry) => entry.kind === "error");
        assert.match(error?.text ?? "", /Coding Workspace is disabled/, JSON.stringify(api.testLog()));
        assert.ok(!api.testLog().some((entry) => entry.kind === "markdown"), "an answer was produced while disabled");
        assert.match(sidebarTurn.bubbles.map((bubble) => bubble.text).join("\n"), /Coding Workspace is disabled/);
        return { askError: error.text, sidebar: sidebarTurn.bubbles };
      } finally {
        restored = restore();
        assert.equal(restored.codingEnabled, true, "setting not restored");
      }
    });
  }

  fs.writeFileSync(reportPath, JSON.stringify({ results, complete: true }, null, 2));
  const failed = results.filter((result) => !result.ok);
  if (failed.length) throw new Error(`${failed.length} UI step(s) failed: ${failed.map((f) => f.name).join("; ")}`);
};
