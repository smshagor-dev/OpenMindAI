/* global fetch */
import fs from "node:fs";
import { WebSocket } from "ws";

const [portArg, fixturePath, outputPath] = process.argv.slice(2);
const port = Number(portArg);
if (!Number.isInteger(port) || !fixturePath || !outputPath) throw new Error("Usage: node export-test6-native-draft.mjs <process-owned-webview-port> <fixture.json> <output.json>");
const { runId } = JSON.parse(fs.readFileSync(fixturePath, "utf8"));
const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
if (pages.filter((page) => page.type === "page").length !== 1) throw new Error("Expected exactly one process-owned native WebView.");
const page = pages.find((target) => target.type === "page");
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.once("open", resolve); socket.once("error", reject); });
let nextId = 0;
const call = (method, params = {}) => new Promise((resolve, reject) => {
  const id = ++nextId;
  const listener = (raw) => {
    const message = JSON.parse(raw.toString());
    if (message.id !== id) return;
    socket.off("message", listener);
    if (message.error) reject(new Error(JSON.stringify(message.error)));
    else resolve(message.result);
  };
  socket.on("message", listener);
  socket.send(JSON.stringify({ id, method, params }));
});
await call("Runtime.enable");
const expression = `Promise.all(["pdf", "docx"].map(async (format) => {
  try {
    const artifact = await window.__TAURI_INTERNALS__.invoke("export_local_research_document", { runId: ${JSON.stringify(runId)}, format, allowDraft: true });
    return { format, ok: true, artifact };
  } catch (error) { return { format, ok: false, error: String(error) }; }
}))`;
const evaluated = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
const result = evaluated.result?.value;
if (!Array.isArray(result)) throw new Error(JSON.stringify(evaluated));
fs.writeFileSync(outputPath, `${JSON.stringify({ capturedAtUtc: new Date().toISOString(), processOwnedWebView: { pid: process.env.OPENMINDAI_PID ?? null, pageId: page.id, url: page.url }, runId, results: result }, null, 2)}\n`);
console.log(JSON.stringify(result, null, 2));
socket.close();
