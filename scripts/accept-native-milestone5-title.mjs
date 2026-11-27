/* global fetch */
import fs from "node:fs";
import { Buffer } from "node:buffer";
import { clearTimeout, setTimeout } from "node:timers";
import { WebSocket } from "ws";

const [portValue, evidenceCsv, jsonPath, formScreenshotPath, proposalScreenshotPath] = process.argv.slice(2);
const port = Number(portValue);
if (!Number.isInteger(port) || !evidenceCsv || !jsonPath || !formScreenshotPath || !proposalScreenshotPath) {
  throw new Error("Usage: node accept-native-milestone5-title.mjs <owned-port> <evidence-csv> <result.json> <form.png> <proposal.png>");
}
const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const pages = targets.filter((target) => target.type === "page");
if (pages.length !== 1) throw new Error(`Expected one process-owned native page; found ${pages.length}`);
const page = pages[0];
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.once("open", resolve); socket.once("error", reject); });
let nextId = 0;
const pending = new Map();
socket.on("message", (raw) => {
  const message = JSON.parse(raw.toString());
  const waiter = pending.get(message.id);
  if (!waiter) return;
  pending.delete(message.id);
  clearTimeout(waiter.timer);
  if (message.error) waiter.reject(new Error(JSON.stringify(message.error)));
  else waiter.resolve(message.result);
});
const call = (method, params = {}, timeoutMs = 30_000) => new Promise((resolve, reject) => {
  const id = ++nextId;
  const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, timeoutMs);
  pending.set(id, { resolve, reject, timer });
  socket.send(JSON.stringify({ id, method, params }));
});
const evaluate = async (expression, timeoutMs = 30_000) => {
  const result = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }, timeoutMs);
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
  return result.result?.value;
};
const writeScreenshot = async (path) => {
  const result = await call("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
  fs.writeFileSync(path, Buffer.from(result.data, "base64"));
};

await call("Runtime.enable");
await call("Page.enable");
const evidenceLiteral = JSON.stringify(evidenceCsv);
const setup = await evaluate(`(async()=>{
  const wait=async(test,limit=10000)=>{const end=Date.now()+limit;while(Date.now()<end){if(test())return true;await new Promise(r=>setTimeout(r,150));}return false;};
  const invoke=window.__TAURI_INTERNALS__.invoke;
  if(typeof invoke!=='function')throw new Error('Native Tauri invoke bridge is unavailable');
  const nav=[...document.querySelectorAll('button,[role=button],a')].find(x=>x.innerText.trim()==='Research Studio');
  if(!nav)throw new Error('Research Studio navigation was not found');
  nav.click(); await new Promise(r=>setTimeout(r,600));
  const fresh=[...document.querySelectorAll('button')].find(x=>x.innerText.trim()==='New local research');
  fresh?.click(); await new Promise(r=>setTimeout(r,300));
  const prompt=document.querySelector('textarea[aria-label="Research Prompt (Required)"]');
  const title=document.querySelector('input[aria-label="Research Title (Optional)"]');
  if(!prompt||!title)throw new Error('Research creation controls did not load');
  const setValue=(element,value,prototype)=>{const setter=Object.getOwnPropertyDescriptor(prototype,'value').set;setter.call(element,value);element.dispatchEvent(new Event('input',{bubbles:true}));element.dispatchEvent(new Event('change',{bubbles:true}));};
  setValue(title,'',HTMLInputElement.prototype);
  setValue(prompt,'Prepare a descriptive, evidence-limited comparison of FedAvg and personalized Per-FedAvg across one matched MNIST IID seed. Keep the Per-FedAvg global reference separate from deployed personalized outcomes. Do not make causal, significance, independent-seed, non-IID, or cross-dataset claims.',HTMLTextAreaElement.prototype);
  const pageSelect=[...document.querySelectorAll('.rs-new select')].find(x=>[...x.options].some(o=>o.textContent.includes('24 pages')));
  if(pageSelect){pageSelect.value=[...pageSelect.options].find(o=>o.textContent.includes('24 pages')).value;pageSelect.dispatchEvent(new Event('change',{bubbles:true}));}
  const route=[...document.querySelectorAll('.rs-new select')].find(x=>[...x.options].some(o=>o.value==='local'));
  if(route){route.value='local';route.dispatchEvent(new Event('change',{bubbles:true}));}
  const models=await invoke('discover_models');
  const model=models.find(x=>x.enabled&&x.path.replaceAll('\\\\','/').toLowerCase().endsWith('qwen3-4b-q4_k_m.gguf'));
  if(!model)throw new Error('The expected local Qwen model is not registered and file-valid');
  const save=[...document.querySelectorAll('.rs-actions button')].find(x=>x.innerText.includes('Save Draft'));
  if(!save||save.disabled)throw new Error('Save Draft was not enabled');
  save.click();
  await wait(()=>[...document.querySelectorAll('[role=status]')].some(x=>x.innerText.includes('Research draft and target page count saved')));
  const projects=await invoke('list_local_research_projects');
  const project=projects.find(x=>x.title==='Untitled research'&&x.prompt.startsWith('Prepare a descriptive, evidence-limited comparison'));
  if(!project)throw new Error('Blank-title project did not persist as Untitled research');
  const imported=await invoke('import_local_research_document',{projectId:project.id,path:${evidenceLiteral}});
  const documents=await invoke('list_local_research_documents',{projectId:project.id});
  const projectButton=[...document.querySelectorAll('.rs-project-title')].find(x=>x.innerText.trim()==='Untitled research');
  projectButton?.click();
  await wait(()=>document.body.innerText.includes(imported.name));
  const generate=[...document.querySelectorAll('.rs-title-proposal button')].find(x=>x.innerText.includes('Generate local title proposal'));
  if(!generate||generate.disabled)throw new Error('Local title proposal button is unavailable or disabled');
  return {project,model,imported,documents,modelReadiness:document.querySelector('.rs-readiness')?.innerText??null,body:document.body.innerText.slice(0,5000)};
})()`, 30_000);
await writeScreenshot(formScreenshotPath);

const generation = await evaluate(`(async()=>{
  const wait=async(test,limit=180000)=>{const end=Date.now()+limit;while(Date.now()<end){if(test())return true;await new Promise(r=>setTimeout(r,200));}return false;};
  const generate=[...document.querySelectorAll('.rs-title-proposal button')].find(x=>x.innerText.includes('Generate local title proposal'));
  generate.click();
  const completed=await wait(()=>document.querySelector('.rs-title-proposal button')?.innerText.includes('Accept reviewed title')||Boolean(document.querySelector('.rs-error')));
  if(!completed)throw new Error('Local title inference did not finish within 180 seconds');
  const proposalInput=document.querySelector('input[aria-label="Review or edit title proposal"]');
  const error=document.querySelector('.rs-error')?.innerText??null;
  if(proposalInput)await window.__TAURI_INTERNALS__.invoke('accept_local_research_title',{jobId:(await window.__TAURI_INTERNALS__.invoke('get_local_research_title_job',{projectId:${JSON.stringify(setup.project.id)}})).id,acceptedTitle:proposalInput.value,expectedCurrentTitle:'Untitled research'});
  const job=await window.__TAURI_INTERNALS__.invoke('get_local_research_title_job',{projectId:${JSON.stringify(setup.project.id)}});
  const projects=await window.__TAURI_INTERNALS__.invoke('list_local_research_projects');
  const project=projects.find(x=>x.id===${JSON.stringify(setup.project.id)});
  return {job,project,titleInput:document.querySelector('input[aria-label="Research Title (Optional)"]')?.value??null,error,acceptanceNotice:[...document.querySelectorAll('[role=status]')].map(x=>x.innerText).find(x=>x.includes('reviewed title was saved'))??null,body:document.body.innerText.slice(0,6000)};
})()`, 210_000);
await writeScreenshot(proposalScreenshotPath);
const output = { capturedAtUtc: new Date().toISOString(), nativePage: { id: page.id, url: page.url }, setup, generation };
fs.writeFileSync(jsonPath, `${JSON.stringify(output, null, 2)}\n`);
console.log(JSON.stringify({ capturedAtUtc: output.capturedAtUtc, projectId: setup.project.id, model: setup.model, imported: setup.imported, titleJob: generation.job, acceptedTitle: generation.project?.title, inferenceError: generation.error, acceptanceNotice: generation.acceptanceNotice }, null, 2));
socket.close();
