import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { WebSocket } from "ws";

const [portText, fixtureFile, outputFile, figureFile] = process.argv.slice(2);
const port = Number(portText);
if (!Number.isInteger(port) || !fixtureFile || !outputFile || !figureFile) throw new Error("Usage: node accept-test6-3c-native-figure.mjs <port> <fixture.json> <output.json> <figure.svg>");
const fixture = JSON.parse(fs.readFileSync(fixtureFile, "utf8"));
const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = targets.find((target) => target.type === "page");
if (!page || targets.filter((target) => target.type === "page").length !== 1) throw new Error("Expected exactly one desktop-owned WebView page.");
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.once("open", resolve); socket.once("error", reject); });
let id = 0;
const cdp = (method, params = {}) => new Promise((resolve, reject) => {
  const requestId = ++id;
  const listener = (raw) => { const message = JSON.parse(raw.toString()); if (message.id !== requestId) return; socket.off("message", listener); message.error ? reject(new Error(JSON.stringify(message.error))) : resolve(message.result); };
  socket.on("message", listener); socket.send(JSON.stringify({ id: requestId, method, params }));
});
await cdp("Runtime.enable");
async function evaluate(expression) {
  const result = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text ?? JSON.stringify(result.exceptionDetails));
  return result.result?.value;
}
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const ui = await evaluate(`(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  if (document.body.innerText.includes("Manuscript proposal")) return { page: location.href, title: document.title, alreadyOpen: true };
  const clickText = async (selector, text) => {
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) { const element = [...document.querySelectorAll(selector)].find((node) => node.innerText.trim() === text || node.innerText.includes(text)); if (element) { element.click(); return true; } await new Promise(r => setTimeout(r, 150)); }
    throw new Error("UI element not found: " + text);
  };
  await clickText("button,a,[role=button]", "Research Studio");
  await clickText("button.rs-project-title", ${JSON.stringify(fixture.title)});
  await clickText("button.rs-project-title", "completed");
  return { page: location.href, title: document.title };
})()`);
await wait(400);
const sourceSha = crypto.createHash("sha256").update(fs.readFileSync(figureFile)).digest("hex");
const existingFigures = await evaluate(`window.__TAURI_INTERNALS__.invoke("list_local_research_figure_assets", { projectId: ${JSON.stringify(fixture.projectId)} })`);
const figure = existingFigures.find((asset) => asset.contentSha256 === sourceSha && asset.caption === "Figure 1. Held-out client accuracy by round") ?? await evaluate(`window.__TAURI_INTERNALS__.invoke("register_local_research_figure", ${JSON.stringify({ projectId: fixture.projectId, sourceRunId: fixture.runId, path: figureFile, caption: "Figure 1. Held-out client accuracy by round", altText: "FedAvg and Per-FedAvg deployed client accuracy across 20 clients; the two runs share an IID MNIST partition and seed 11." })})`);
if (!figure?.id || figure.projectId !== fixture.projectId) throw new Error(`Figure was not registered to the disposable project: ${JSON.stringify(figure)}`);
const inserted = await evaluate(`(async () => {
  const clickText = async (selector, text) => { const deadline = Date.now() + 30000; while (Date.now() < deadline) { const el = [...document.querySelectorAll(selector)].find(n => n.innerText.includes(text)); if (el) { el.click(); return true; } await new Promise(r => setTimeout(r, 150)); } throw new Error("Missing UI text: " + text); };
  await clickText("button", "Insert into manuscript draft");
  const editor = document.querySelector('textarea[aria-label="Edit manuscript revision"]');
  if (!editor || !editor.value.includes("research-figure:${figure.id}")) throw new Error("Canonical project figure reference was not inserted by Research Studio.");
  const canonical = ${JSON.stringify(`![${figure.altText}](research-figure:${figure.id} "${figure.caption}")`)};
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value").set;
  const nl = String.fromCharCode(10); const cleaned = editor.value.split(nl).filter(line => !line.includes("figure-1-client-accuracy.svg") && !line.includes("research-figure:${figure.id}")).join(nl).trimEnd() + nl + nl + canonical + nl;
  setter.call(editor, cleaned);
  editor.dispatchEvent(new Event("input", { bubbles: true }));
  if (!editor.value.includes(canonical) || editor.value.includes("figure-1-client-accuracy.svg")) throw new Error("Relink diagnostic: " + JSON.stringify({ canonical, hasCanonical: editor.value.includes(canonical), hasLegacy: editor.value.includes("figure-1-client-accuracy.svg"), tail: editor.value.slice(-600) }));
  await clickText("button", "Save as new revision");
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) { const text = [...document.querySelectorAll(".rs-manuscript-review")].map(n => n.innerText).join("\\n"); if (text.includes("research-figure:${figure.id}") && !document.querySelector('textarea[aria-label="Edit manuscript revision"]')) return { text, reference: "research-figure:${figure.id}" }; await new Promise(r => setTimeout(r, 150)); }
  throw new Error("Saved manuscript revision did not contain the canonical figure reference.");
})()`);
const screenshot = async (name) => {
  const shot = await cdp("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
  const dest = path.join(path.dirname(outputFile), name);
  fs.writeFileSync(dest, Buffer.from(shot.data, "base64"));
  return dest;
};
const linkedScreenshot = await screenshot("native-figure-linked.png");
const exported = await evaluate(`(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const before = await invoke("list_artifacts", { conversationId: ${JSON.stringify(fixture.conversationId)} });
  const ids = new Set(before.map(item => item.id));
  const make = async (label) => {
    const button = [...document.querySelectorAll("button")].find(n => n.innerText.trim() === label);
    if (!button) throw new Error("Missing export workflow button: " + label);
    button.click();
    const end = Date.now() + 120000;
    while (Date.now() < end) {
      const rows = await invoke("list_artifacts", { conversationId: ${JSON.stringify(fixture.conversationId)} });
      const item = rows.find(r => !ids.has(r.id) && r.status === "ready" && r.kind === (label.includes("PDF") ? "pdf" : "docx"));
      if (item) { ids.add(item.id); return item; }
      const error = [...document.querySelectorAll("[role=alert]")].map(n => n.innerText).filter(Boolean).join("\\n");
      if (error) throw new Error(error);
      await new Promise(r => setTimeout(r, 250));
    }
    throw new Error("Timed out waiting for " + label);
  };
  return [await make("Export draft PDF"), await make("Export draft DOCX")];
})()`);
const rootInfo = await evaluate(`window.__TAURI_INTERNALS__.invoke("get_portable_root")`);
const root = rootInfo.root;
const artifacts = exported.map((artifact) => {
  const fullPath = path.join(root, artifact.path);
  const bytes = fs.readFileSync(fullPath);
  return { ...artifact, fullPath, fileSha256: crypto.createHash("sha256").update(bytes).digest("hex"), actualSizeBytes: bytes.length };
});
const record = { capturedAtUtc: new Date().toISOString(), processOwnedPage: { id: page.id, url: page.url }, fixture, registration: figure, canonicalReference: `research-figure:${figure.id}`, linkedScreenshot, manuscript: inserted, root, exports: artifacts };
fs.writeFileSync(outputFile, `${JSON.stringify(record, null, 2)}\n`);
console.log(JSON.stringify(record, null, 2));
socket.close();
