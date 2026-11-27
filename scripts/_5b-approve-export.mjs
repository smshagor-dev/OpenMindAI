/* global fetch */
import fs from "node:fs";
import { Buffer } from "node:buffer";
import { WebSocket } from "ws";

const [portValue, outDir] = process.argv.slice(2);
const port = Number(portValue);
if (!Number.isInteger(port) || !outDir) throw new Error("Usage: node _5b-approve-export.mjs <port> <outDir>");
fs.mkdirSync(outDir, { recursive: true });

const pages = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t => t.type === "page");
const socket = new WebSocket(pages[0].webSocketDebuggerUrl);
await new Promise((res, rej) => { socket.once("open", res); socket.once("error", rej); });
let id = 0; const pending = new Map();
socket.on("message", raw => { const m = JSON.parse(raw); const w = pending.get(m.id); if (!w) return; pending.delete(m.id); m.error ? w.rej(new Error(JSON.stringify(m.error))) : w.res(m.result); });
const call = (method, params = {}, timeoutMs = 120000) => new Promise((res, rej) => { const i = ++id; const t = setTimeout(() => { pending.delete(i); rej(new Error(`${method} timed out`)); }, timeoutMs); pending.set(i, { res: v => { clearTimeout(t); res(v); }, rej: e => { clearTimeout(t); rej(e); } }); socket.send(JSON.stringify({ id: i, method, params })); });
const ev = async (expr, timeoutMs = 120000) => { const r = await call("Runtime.evaluate", { expression: `(async()=>{${expr}})()`, awaitPromise: true, returnByValue: true }, timeoutMs); if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? JSON.stringify(r.exceptionDetails)); return r.result?.value; };
const shot = async name => { const r = await call("Page.captureScreenshot", { format: "png", captureBeyondViewport: true }); fs.writeFileSync(`${outDir}/${name}.png`, Buffer.from(r.data, "base64")); };
await call("Runtime.enable"); await call("Page.enable");

// Enter the project view fresh.
await ev(`
  const wait=async(t,l=15000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,150));}return false;};
  const n=[...document.querySelectorAll('button,[role=button],a')].find(x=>x.innerText.trim()==='Research Studio'); n&&n.click(); await new Promise(r=>setTimeout(r,500));
  const b=[...document.querySelectorAll('.rs-project-title')].find(x=>x.innerText.includes('FedAvg')); b&&b.click();
  await wait(()=>Boolean(document.querySelector('.rs-manuscript-workbench')));
  return true;
`);

// Phase: approval concurrency. Approve Results via the GUI button; prove a stale
// hash is rejected; prove a human edit creates a new unapproved revision that
// does not silently become the approved/exported one.
const approval = await ev(`
  const wait=async(t,l=20000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,150));}return false;};
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const proj=(await invoke('list_local_research_projects'))[0];
  const before=(await invoke('list_local_research_sections',{projectId:proj.id})).filter(s=>s.sectionKey==='results').sort((a,b)=>b.revision-a.revision)[0];
  // Stale-hash approval must fail before we approve for real.
  let staleRejected=false;
  try { await invoke('approve_local_research_section',{draftId:before.id,expectedContentSha256:'0'.repeat(64)}); } catch(e){ staleRejected=true; }
  // Approve via the visible GUI button.
  const approveBtn=[...document.querySelectorAll('.rs-draft button')].find(b=>b.innerText.trim()==='Approve section');
  if(!approveBtn) throw new Error('Approve button not found');
  approveBtn.click();
  await wait(()=>{const s=document.querySelector('.rs-export')?.innerText||''; return /1 approved section/.test(s);});
  const afterApprove=(await invoke('list_local_research_sections',{projectId:proj.id})).filter(s=>s.sectionKey==='results');
  const approved=afterApprove.find(s=>s.status==='approved');
  // Re-approving the now-approved (non-draft) revision must fail.
  let reApproveRejected=false;
  try { await invoke('approve_local_research_section',{draftId:approved.id,expectedContentSha256:approved.contentSha256}); } catch(e){ reApproveRejected=true; }
  // Human-edit: append a sentence with a VERIFIED fact value only; this creates
  // a new draft revision and must NOT change the approved revision.
  const editBtn=[...document.querySelectorAll('.rs-draft button')].find(b=>b.innerText.trim()==='Edit');
  editBtn.click();
  await wait(()=>Boolean(document.querySelector('.rs-draft textarea')));
  const ta=document.querySelector('.rs-draft textarea');
  const setter=Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set;
  setter.call(ta, ta.value.trimEnd()+'\\n\\nFedAvg evaluated 10000 held-out images in this seed.');
  ta.dispatchEvent(new Event('input',{bubbles:true}));
  const saveBtn=[...document.querySelectorAll('.rs-draft button')].find(b=>/Save as new revision/.test(b.innerText));
  saveBtn.click();
  await wait(()=>{const secs=document.querySelector('.rs-drafts')?.innerText||''; return /human-edit/.test(secs);});
  const afterEdit=(await invoke('list_local_research_sections',{projectId:proj.id})).filter(s=>s.sectionKey==='results');
  const approvedStill=afterEdit.find(s=>s.status==='approved');
  return {
    staleRejected, reApproveRejected,
    approvedRevision: approved.revision, approvedHash: approved.contentSha256,
    approvedUnchangedAfterEdit: approvedStill && approvedStill.revision===approved.revision,
    revisions: afterEdit.map(s=>({rev:s.revision,status:s.status,origin:s.origin}))
  };
`);
await shot("05-approved-and-edited");

// Phase: export the approved manuscript to PDF and DOCX via the GUI buttons.
const exportPdf = await ev(`
  const wait=async(t,l=120000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,200));}return false;};
  const btn=[...document.querySelectorAll('.rs-export button')].find(b=>b.innerText.trim()==='Export approved PDF');
  if(!btn||btn.disabled) throw new Error('Export approved PDF button unavailable');
  btn.click();
  const ok=await wait(()=>{const s=document.querySelector('.rs-export-result')?.innerText||''; return /Status:/.test(s) || Boolean(document.querySelector('.rs-manuscript-workbench .rs-error'));});
  if(!ok) throw new Error('PDF export did not report a result');
  const err=document.querySelector('.rs-manuscript-workbench .rs-error')?.innerText||null;
  return {resultText: document.querySelector('.rs-export-result')?.innerText||null, error: err};
`, 180000);
await shot("06-export-pdf-result");

const exportDocx = await ev(`
  const wait=async(t,l=120000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,200));}return false;};
  const btn=[...document.querySelectorAll('.rs-export button')].find(b=>b.innerText.trim()==='Export approved DOCX');
  btn.click();
  await wait(()=>{const s=document.querySelector('.rs-export-result')?.innerText||''; return /DOCX/.test(s) || Boolean(document.querySelector('.rs-manuscript-workbench .rs-error'));});
  return {resultText: document.querySelector('.rs-export-result')?.innerText||null};
`, 180000);
await shot("07-export-docx-result");

const output = { capturedAtUtc: new Date().toISOString(), approval, exportPdf, exportDocx };
fs.writeFileSync(`${outDir}/approve-export.json`, JSON.stringify(output, null, 2) + "\n");
console.log(JSON.stringify(output, null, 2));
socket.close();
