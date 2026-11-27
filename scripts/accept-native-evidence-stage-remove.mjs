/* global fetch */
import fs from "node:fs";
import { WebSocket } from "ws";

const [portArg, evidencePath, outputPath] = process.argv.slice(2);
const port = Number(portArg);
if (!Number.isInteger(port) || !evidencePath || !outputPath) throw new Error("Usage: node accept-native-evidence-stage-remove.mjs <process-owned-webview-port> <local-evidence-file> <output.json>");
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
const expression = `(async () => {
  const nav=[...document.querySelectorAll('button,[role=button],a')].find(x=>x.innerText.trim()==='Research Studio'); nav?.click(); await new Promise(r=>setTimeout(r,500));
  const area=document.querySelector('.rs-evidence-drop'); if(!area) throw new Error('Evidence staging area is missing');
  const file=new File(['staging-only'],${JSON.stringify(evidencePath.split(/[\\/]/).pop())},{type:'text/markdown'}); Object.defineProperty(file,'path',{value:${JSON.stringify(evidencePath)}});
  const transfer=new DataTransfer(); transfer.items.add(file); area.dispatchEvent(new DragEvent('drop',{bubbles:true,cancelable:true,dataTransfer:transfer})); await new Promise(r=>setTimeout(r,300));
  const staged=[...area.querySelectorAll('.rs-evidence-pending')]; const before=staged.map(x=>x.innerText); const remove=staged[0]?.querySelector('button'); remove?.click(); await new Promise(r=>setTimeout(r,150));
  return {dropMethod:'synthetic DOM drop with native-style file.path; not OS drag/drop',stagedBeforeRemoval:before,stagedAfterRemoval:[...area.querySelectorAll('.rs-evidence-pending')].map(x=>x.innerText),importedProjectDocuments:(await window.__TAURI_INTERNALS__.invoke('list_local_research_documents',{projectId:'03b4edfb-c985-41e5-b128-33c01f21024c'})).length};
})()`;
const evaluated = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
const result = evaluated.result?.value;
if (!result) throw new Error(JSON.stringify(evaluated));
const output={capturedAtUtc:new Date().toISOString(),processOwnedPage:{id:page.id,url:page.url},result};
fs.writeFileSync(outputPath,`${JSON.stringify(output,null,2)}\n`); console.log(JSON.stringify(output,null,2)); socket.close();
