/* global fetch */
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { WebSocket } from "ws";

const [portArg, fixturePath, resultPath, outputPath] = process.argv.slice(2);
const port = Number(portArg);
if (!Number.isInteger(port) || !fixturePath || !resultPath || !outputPath) throw new Error("Usage: node verify-test6-native-restart.mjs <port> <fixture.json> <export-results.json> <output.json>");
const fixture = JSON.parse(fs.readFileSync(fixturePath, "utf8"));
const exports = JSON.parse(fs.readFileSync(resultPath, "utf8")).results;
const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = pages.find((target) => target.type === "page");
if (!page || pages.filter((target) => target.type === "page").length !== 1) throw new Error("Expected one process-owned native WebView.");
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.once("open", resolve); socket.once("error", reject); });
let id = 0;
const call = (method, params = {}) => new Promise((resolve, reject) => {
  const requestId = ++id;
  const listener = (raw) => { const message = JSON.parse(raw.toString()); if (message.id !== requestId) return; socket.off("message", listener); message.error ? reject(new Error(JSON.stringify(message.error))) : resolve(message.result); };
  socket.on("message", listener); socket.send(JSON.stringify({ id: requestId, method, params }));
});
await call("Runtime.enable");
const names = ["get_portable_root", "list_local_research_projects", "list_local_research_documents", "list_local_research_runs", "list_artifacts"];
const args = [undefined, undefined, { projectId: "03b4edfb-c985-41e5-b128-33c01f21024c" }, { projectId: fixture.projectId }, { conversationId: fixture.conversationId }];
const expression = `(async () => { const out={}; for (const [i,name] of ${JSON.stringify(names)}.entries()) { try { out[name]=await window.__TAURI_INTERNALS__.invoke(name,${JSON.stringify(args)}[i]); } catch(error) { out[name]={error:String(error)}; } } return out; })()`;
const evaluated = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
const native = evaluated.result?.value;
if (!native) throw new Error(JSON.stringify(evaluated));
const root = native.get_portable_root.root;
const artifactRows = Array.isArray(native.list_artifacts) ? native.list_artifacts : [];
const exported = exports.map(({ format, artifact }) => {
  const file = path.join(root, artifact.path);
  return { format, id: artifact.id, name: artifact.name, exists: fs.existsSync(file), sizeBytes: fs.existsSync(file) ? fs.statSync(file).size : null, sha256: fs.existsSync(file) ? crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex") : null, listedAfterRestart: artifactRows.some((row) => row.id === artifact.id) };
});
const report = {
  capturedAtUtc: new Date().toISOString(),
  processOwnedWebView: { id: page.id, url: page.url },
  native,
  evidenceProject: { id: "03b4edfb-c985-41e5-b128-33c01f21024c", documentCount: Array.isArray(native.list_local_research_documents) ? native.list_local_research_documents.length : null },
  exportProjectPresent: Array.isArray(native.list_local_research_projects) && native.list_local_research_projects.some((item) => item.id === fixture.projectId),
  draftRunPresent: Array.isArray(native.list_local_research_runs) && native.list_local_research_runs.some((item) => item.id === fixture.runId),
  exportsAfterRestart: exported,
};
fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify(report, null, 2));
socket.close();
