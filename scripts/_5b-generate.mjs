/* global fetch */
import fs from "node:fs";
import { Buffer } from "node:buffer";
import { setTimeout as sleep } from "node:timers/promises";
import { WebSocket } from "ws";

const [portValue, evidenceDir, outDir] = process.argv.slice(2);
const port = Number(portValue);
if (!Number.isInteger(port) || !evidenceDir || !outDir) throw new Error("Usage: node _5b-generate.mjs <port> <evidenceDir> <outDir>");
fs.mkdirSync(outDir, { recursive: true });

const pages = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t => t.type === "page");
if (pages.length !== 1) throw new Error(`expected 1 page, found ${pages.length}`);
const socket = new WebSocket(pages[0].webSocketDebuggerUrl);
await new Promise((res, rej) => { socket.once("open", res); socket.once("error", rej); });
let id = 0; const pending = new Map();
socket.on("message", raw => { const m = JSON.parse(raw); const w = pending.get(m.id); if (!w) return; pending.delete(m.id); m.error ? w.rej(new Error(JSON.stringify(m.error))) : w.res(m.result); });
const call = (method, params = {}, timeoutMs = 60000) => new Promise((res, rej) => { const i = ++id; const t = setTimeout(() => { pending.delete(i); rej(new Error(`${method} timed out`)); }, timeoutMs); pending.set(i, { res: v => { clearTimeout(t); res(v); }, rej: e => { clearTimeout(t); rej(e); } }); socket.send(JSON.stringify({ id: i, method, params })); });
const ev = async (expr, timeoutMs = 60000) => { const r = await call("Runtime.evaluate", { expression: `(async()=>{${expr}})()`, awaitPromise: true, returnByValue: true }, timeoutMs); if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? JSON.stringify(r.exceptionDetails)); return r.result?.value; };
const shot = async name => { const r = await call("Page.captureScreenshot", { format: "png", captureBeyondViewport: false }); fs.writeFileSync(`${outDir}/${name}.png`, Buffer.from(r.data, "base64")); };

await call("Runtime.enable"); await call("Page.enable");

const tableCsv = `${evidenceDir}/table-1-heldout-metrics.csv`.replaceAll("/", "\\");
const analysisJson = `${evidenceDir}/analysis.json`.replaceAll("/", "\\");

// Step 1: navigate and import canonical evidence through the SAME registered
// command the native chooser calls (chooser yields a path; UI then invokes this).
// Import BEFORE selecting the project so the manuscript component mounts fresh
// with the new evidence already indexed.
const setup = await ev(`
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const nav=[...document.querySelectorAll('button,[role=button],a')].find(x=>x.innerText.trim()==='Research Studio');
  if(!nav) throw new Error('Research Studio nav not found'); nav.click(); await new Promise(r=>setTimeout(r,600));
  const proj=(await invoke('list_local_research_projects'))[0];
  const imported1=await invoke('import_local_research_document',{projectId:proj.id,path:${JSON.stringify(tableCsv)}});
  const imported2=await invoke('import_local_research_document',{projectId:proj.id,path:${JSON.stringify(analysisJson)}});
  const dup=await invoke('import_local_research_document',{projectId:proj.id,path:${JSON.stringify(tableCsv)}});
  return {projectId:proj.id, savedPageTarget:proj.manuscript_page_target, imported1, imported2, dedupSameId: dup.id===imported1.id, dedupSameSha: dup.contentSha256===imported1.contentSha256};
`);
await shot("01-studio-open");

// Reload so the manuscript workbench mounts fresh and re-fetches the evidence
// that was just imported (it is keyed by project id and will not otherwise
// re-run its initial fetch for an already-selected project).
await call("Page.reload", { ignoreCache: false });
await sleep(1500);
await ev(`const w=async(t,l=20000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,150));}return false;}; await w(()=>typeof window.__TAURI_INTERNALS__!=='undefined'&&[...document.querySelectorAll('button,[role=button],a')].some(x=>x.innerText.trim()==='Research Studio')); return true;`);

// Step 2: navigate, select the project (mounts the manuscript workbench fresh),
// then classify the metrics table as derived_analysis through the UI <select>.
const classified = await ev(`
  const wait=async(t,l=20000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,150));}return false;};
  const nav=[...document.querySelectorAll('button,[role=button],a')].find(x=>x.innerText.trim()==='Research Studio'); nav.click(); await new Promise(r=>setTimeout(r,600));
  const btn=[...document.querySelectorAll('.rs-project-title')].find(x=>x.innerText.includes('FedAvg'));
  if(!btn) throw new Error('project button not found'); btn.click();
  await wait(()=>Boolean(document.querySelector('.rs-manuscript-workbench')));
  await wait(()=>[...document.querySelectorAll('.rs-manuscript-workbench li strong')].some(s=>s.innerText.includes('table-1-heldout-metrics.csv')));
  const details=document.querySelector('.rs-evidence-classes');
  if(details && !details.open){details.open=true;}
  await wait(()=>[...document.querySelectorAll('.rs-manuscript-workbench li strong')].some(s=>s.innerText.includes('table-1-heldout-metrics.csv')));
  const li=[...document.querySelectorAll('.rs-manuscript-workbench li')].find(x=>x.querySelector('strong')?.innerText.includes('table-1-heldout-metrics.csv'));
  if(!li) throw new Error('metrics csv row not found in classify UI');
  const sel=li.querySelector('select');
  const setter=Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set;
  setter.call(sel,'derived_analysis'); sel.dispatchEvent(new Event('change',{bubbles:true}));
  await wait(()=>{const l2=[...document.querySelectorAll('.rs-manuscript-workbench li')].find(x=>x.querySelector('strong')?.innerText.includes('table-1-heldout-metrics.csv')); return l2 && /[1-9][0-9]* numeric fact/.test(l2.innerText);});
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const proj=(await invoke('list_local_research_projects'))[0];
  const facts=await invoke('local_research_evidence_facts',{projectId:proj.id});
  return {numericFacts:facts.facts.length, classifiedDocs:facts.documents.filter(d=>d.sourceClass).map(d=>({name:d.name,class:d.sourceClass,facts:d.factCount}))};
`);
await shot("02-classified");

// Step 3: generate the Results section via the visible UI button (true GUI generation).
const t0 = Date.now();
const generated = await ev(`
  const wait=async(t,l=600000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,400));}return false;};
  const rowFor=key=>[...document.querySelectorAll('.rs-section-table tbody tr')].find(tr=>tr.firstElementChild?.innerText.trim()==='Experimental Setup and Methods'||tr.firstElementChild?.innerText.trim()==='Results');
  const resultsRow=[...document.querySelectorAll('.rs-section-table tbody tr')].find(tr=>tr.firstElementChild?.innerText.trim()==='Results');
  if(!resultsRow) throw new Error('Results row not found');
  const genBtn=[...resultsRow.querySelectorAll('button')].find(b=>/Generate|Regenerate/.test(b.innerText));
  if(!genBtn) throw new Error('Results Generate button not found');
  if(genBtn.disabled) throw new Error('Results Generate button disabled (model not ready?)');
  genBtn.click();
  const done=await wait(()=>{const r=[...document.querySelectorAll('.rs-section-table tbody tr')].find(tr=>tr.firstElementChild?.innerText.trim()==='Results'); const txt=r?.innerText||''; return /approved|draft r[0-9]|insufficient|failed|cancelled/.test(txt) && !/not started/.test(txt);});
  if(!done) throw new Error('Results generation did not finish in 600s');
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const proj=(await invoke('list_local_research_projects'))[0];
  const sections=await invoke('list_local_research_sections',{projectId:proj.id});
  const results=sections.filter(s=>s.sectionKey==='results').sort((a,b)=>b.revision-a.revision)[0];
  const ledger=await invoke('local_research_claim_ledger',{projectId:proj.id});
  const resultsLedger=ledger.filter(l=>l.sectionKey==='results');
  return {status:results?.status, revision:results?.revision, origin:results?.origin, modelId:results?.modelId, latencyMs:results?.latencyMs, error:results?.error, contentSha256:results?.contentSha256, content:results?.content, validation:results?.validation, ledgerCount:resultsLedger.length, ledgerSample:resultsLedger.slice(0,20).map(l=>({value:l.renderedToken,fact:l.factId,sha:l.documentSha256.slice(0,12),locator:l.locator,method:l.method}))};
`, 650000);
await shot("03-results-generated");

const output = { capturedAtUtc: new Date().toISOString(), wallClockMs: Date.now() - t0, setup, classified, generated };
fs.writeFileSync(`${outDir}/generate-results.json`, JSON.stringify(output, null, 2) + "\n");
console.log(JSON.stringify({ savedPageTarget: setup.savedPageTarget, dedupSameId: setup.dedupSameId, numericFacts: classified.numericFacts, resultStatus: generated.status, revision: generated.revision, modelId: generated.modelId, latencyMs: generated.latencyMs, ledgerCount: generated.ledgerCount, unsupported: generated.validation?.unsupportedTokens, error: generated.error }, null, 2));
socket.close();
