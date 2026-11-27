/* global fetch */
import fs from "node:fs";
import { WebSocket } from "ws";

const [portValue, svgPath, outDir] = process.argv.slice(2);
const port = Number(portValue);
if (!Number.isInteger(port) || !svgPath || !outDir) throw new Error("Usage: node _5b-figure.mjs <port> <svgPath> <outDir>");
fs.mkdirSync(outDir, { recursive: true });
const svgLiteral = JSON.stringify(svgPath.replaceAll("/", "\\"));

const pages = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t => t.type === "page");
const socket = new WebSocket(pages[0].webSocketDebuggerUrl);
await new Promise((res, rej) => { socket.once("open", res); socket.once("error", rej); });
let id = 0; const pending = new Map();
socket.on("message", raw => { const m = JSON.parse(raw); const w = pending.get(m.id); if (!w) return; pending.delete(m.id); m.error ? w.rej(new Error(JSON.stringify(m.error))) : w.res(m.result); });
const call = (method, params = {}, t = 180000) => new Promise((res, rej) => { const i = ++id; const tm = setTimeout(() => { pending.delete(i); rej(new Error(`${method} timed out`)); }, t); pending.set(i, { res: v => { clearTimeout(tm); res(v); }, rej: e => { clearTimeout(tm); rej(e); } }); socket.send(JSON.stringify({ id: i, method, params })); });
const ev = async (expr, t = 180000) => { const r = await call("Runtime.evaluate", { expression: `(async()=>{${expr}})()`, awaitPromise: true, returnByValue: true }, t); if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? JSON.stringify(r.exceptionDetails)); return r.result?.value; };
await call("Runtime.enable");

const out = await ev(`
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const proj=(await invoke('list_local_research_projects'))[0];
  // Register the validated scientific figure (chooser yields a path; UI invokes this command).
  const fig=await invoke('register_local_research_figure',{projectId:proj.id, sourceRunId:null, path:${svgLiteral}, caption:'Figure 1. Per-client held-out accuracy for FedAvg and personalized Per-FedAvg (one MNIST IID seed).', altText:'Grouped bars of per-client held-out accuracy for the two methods.'});
  // Insert the figure into a Data and Code Availability section via a human edit
  // (section pipeline accepts a research-figure: reference; it is resolved and
  // embedded at export, and fails closed if the asset is missing/tampered).
  const refLine='Figure 1 is included below.\\n\\n![Grouped bars of per-client held-out accuracy for the two methods.](research-figure:'+fig.id+' \"Figure 1. Per-client held-out accuracy for FedAvg and personalized Per-FedAvg (one MNIST IID seed).\")';
  const edit=await invoke('save_local_research_section_edit',{projectId:proj.id, sectionKey:'data_availability', baseDraftId:null, content:'## Data and Code Availability\\n\\nThe evidence file table-1-heldout-metrics.csv was used; its SHA-256 is recorded in the claim ledger.\\n\\n'+refLine});
  await invoke('approve_local_research_section',{draftId:edit.id, expectedContentSha256:edit.contentSha256});
  const good=await invoke('export_local_research_section_manuscript',{projectId:proj.id, format:'pdf', allowDraft:false});
  // Fail-closed: tamper the figure reference to a non-existent asset id and re-export.
  const bad=await invoke('save_local_research_section_edit',{projectId:proj.id, sectionKey:'data_availability', baseDraftId:edit.id, content:'## Data and Code Availability\\n\\n![x](research-figure:00000000-0000-0000-0000-000000000000 \"missing\")'});
  await invoke('approve_local_research_section',{draftId:bad.id, expectedContentSha256:bad.contentSha256});
  let failClosed=null;
  try { await invoke('export_local_research_section_manuscript',{projectId:proj.id, format:'pdf', allowDraft:false}); failClosed='EXPORT UNEXPECTEDLY SUCCEEDED'; }
  catch(e){ failClosed=String(e); }
  return {figure:{id:fig.id,name:fig.name,sha256:fig.contentSha256,w:fig.width_px,h:fig.height_px,caption:fig.caption,alt:fig.alt_text}, goodExport:good, failClosed};
`);
fs.writeFileSync(`${outDir}/figure-export.json`, JSON.stringify(out, null, 2) + "\n");
console.log(JSON.stringify(out, null, 2));
socket.close();
