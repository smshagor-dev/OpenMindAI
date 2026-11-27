/* global fetch */
import fs from "node:fs";
import { WebSocket } from "ws";

const [portArg, projectId, outputPath] = process.argv.slice(2);
const port = Number(portArg);
if (!Number.isInteger(port) || !projectId || !outputPath) throw new Error("Usage: node accept-native-missing-model.mjs <process-owned-webview-port> <disposable-project-id> <output.json>");
const page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((target) => target.type === "page");
if (!page) throw new Error("Native WebView page was not found.");
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.once("open", resolve); socket.once("error", reject); });
let id = 0;
const call = (method, params = {}) => new Promise((resolve, reject) => { const requestId = ++id; const listener = (raw) => { const message = JSON.parse(raw.toString()); if (message.id !== requestId) return; socket.off("message", listener); message.error ? reject(new Error(JSON.stringify(message.error))) : resolve(message.result); }; socket.on("message", listener); socket.send(JSON.stringify({ id: requestId, method, params })); });
await call("Runtime.enable");
const expression = `(async()=>{const before=await window.__TAURI_INTERNALS__.invoke('list_local_research_runs',{projectId:${JSON.stringify(projectId)}});let failure=null;try{await window.__TAURI_INTERNALS__.invoke('start_local_research_run',{projectId:${JSON.stringify(projectId)},modelId:null,executionMode:'local',manuscriptTargetWords:3500})}catch(error){failure=error?.message??JSON.stringify(error)}const after=await window.__TAURI_INTERNALS__.invoke('list_local_research_runs',{projectId:${JSON.stringify(projectId)}});return {beforeCount:before.length,afterCount:after.length,blockedError:failure,pass:Boolean(failure&&before.length===after.length&&failure.toLowerCase().includes('install a compatible local model'))}})()`;
const evaluated=await call("Runtime.evaluate",{expression,awaitPromise:true,returnByValue:true});
const output={capturedAtUtc:new Date().toISOString(),processOwnedPageId:page.id,result:evaluated.result?.value};
if(!output.result)throw new Error(JSON.stringify(evaluated));
fs.writeFileSync(outputPath,`${JSON.stringify(output,null,2)}\n`);console.log(JSON.stringify(output,null,2));socket.close();
