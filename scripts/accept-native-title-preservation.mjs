/* global fetch */
import fs from "node:fs";
import { WebSocket } from "ws";

const [portArg, outputPath] = process.argv.slice(2);
const port = Number(portArg);
if (!Number.isInteger(port) || !outputPath) throw new Error("Usage: node accept-native-title-preservation.mjs <process-owned-webview-port> <output.json>");
const page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((target) => target.type === "page");
if (!page) throw new Error("Native WebView page was not found.");
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.once("open", resolve); socket.once("error", reject); });
let id = 0;
const call = (method, params = {}) => new Promise((resolve, reject) => { const requestId = ++id; const listener = (raw) => { const message = JSON.parse(raw.toString()); if (message.id !== requestId) return; socket.off("message", listener); message.error ? reject(new Error(JSON.stringify(message.error))) : resolve(message.result); }; socket.on("message", listener); socket.send(JSON.stringify({ id: requestId, method, params })); });
await call("Runtime.enable");
const expression = `(async () => {
  const target='Test 6 acceptance draft';
  const button=[...document.querySelectorAll('button')].find(x=>x.innerText.includes(target)); button?.click(); await new Promise(r=>setTimeout(r,350));
  const title=document.querySelector('input[aria-label="Research Title (Optional)"]'); const prompt=document.querySelector('textarea[aria-label="Research Prompt (Required)"]');
  const setter=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set; setter.call(title,''); title.dispatchEvent(new Event('input',{bubbles:true}));
  [...document.querySelectorAll('.rs-actions button')].find(x=>x.innerText.includes('Save Draft'))?.click(); await new Promise(r=>setTimeout(r,600));
  const projects=await window.__TAURI_INTERNALS__.invoke('list_local_research_projects'); const saved=projects.find(x=>x.title===target);
  return {existingTitleInputCleared:title.value==='',promptRetained:prompt?.value==='Acceptance-only draft to verify native project settings persist. Do not generate claims or run inference.',savedProjectTitle:saved?.title??null,savedPrompt:saved?.prompt??null};
})()`;
const evaluated = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
const output={capturedAtUtc:new Date().toISOString(),processOwnedPageId:page.id,result:evaluated.result?.value};
if (!output.result) throw new Error(JSON.stringify(evaluated));
fs.writeFileSync(outputPath,`${JSON.stringify(output,null,2)}\n`); console.log(JSON.stringify(output,null,2)); socket.close();
