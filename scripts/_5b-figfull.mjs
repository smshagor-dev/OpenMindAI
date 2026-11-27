import fs from "node:fs"; import { WebSocket } from "ws";
const port=Number(process.argv[2]); const svgPath=process.argv[3].replaceAll("/","\\"); const outDir=process.argv[4];
fs.mkdirSync(outDir,{recursive:true});
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={},t=180000)=>new Promise((r,j)=>{const i=++id;const tm=setTimeout(()=>{p.delete(i);j(new Error(m+" timeout"));},t);p.set(i,{res:v=>{clearTimeout(tm);r(v);},rej:e=>{clearTimeout(tm);j(e);}});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async(e,t=180000)=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true},t);if(r.exceptionDetails)throw new Error(JSON.stringify(r.exceptionDetails.exception??r.exceptionDetails));return r.result?.value;};
await call("Runtime.enable");

const caption="Figure 1. Per-client held-out accuracy for FedAvg and personalized Per-FedAvg (one MNIST IID seed).";
const alt="Grouped bars of per-client held-out accuracy for the two methods.";
// Content strings are built in Node and injected as JSON literals (safe escaping).
const FIG="__FIGID__";
const goodContent=`## Data and Code Availability\n\nThe evidence file table-1-heldout-metrics.csv was used; its SHA-256 is recorded in the claim ledger.\n\nFigure 1 is included below.\n\n![${alt}](research-figure:${FIG} "${caption}")`;
const badContent=`## Data and Code Availability\n\n![missing figure](research-figure:00000000-0000-0000-0000-000000000000 "missing")`;

const out=await ev(`
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const proj=(await invoke('list_local_research_projects'))[0];
  const steps={};
  const fig=await invoke('register_local_research_figure',{projectId:proj.id,sourceRunId:null,path:${JSON.stringify(svgPath)},caption:${JSON.stringify(caption)},altText:${JSON.stringify(alt)}});
  steps.figureId=fig.id; steps.figureSha=fig.contentSha256; steps.figureDims=[fig.width_px,fig.height_px];
  const goodContent=${JSON.stringify(goodContent)}.replace('__FIGID__',fig.id);
  const edit=await invoke('save_local_research_section_edit',{projectId:proj.id,sectionKey:'data_availability',baseDraftId:null,content:goodContent});
  steps.editId=edit.id; steps.editWarnings=edit.validation&&edit.validation.warnings;
  await invoke('approve_local_research_section',{draftId:edit.id,expectedContentSha256:edit.contentSha256});
  const good=await invoke('export_local_research_section_manuscript',{projectId:proj.id,format:'pdf',allowDraft:false});
  steps.goodExport={format:good.format,pages:good.actualPages,status:good.pageTargetStatus,words:good.wordCount,fileSha:good.fileSha256,artifactPath:good.artifact.path,ledgerSha:good.ledgerSha256};
  const bad=await invoke('save_local_research_section_edit',{projectId:proj.id,sectionKey:'data_availability',baseDraftId:edit.id,content:${JSON.stringify(badContent)}});
  await invoke('approve_local_research_section',{draftId:bad.id,expectedContentSha256:bad.contentSha256});
  try{await invoke('export_local_research_section_manuscript',{projectId:proj.id,format:'pdf',allowDraft:false});steps.failClosed='UNEXPECTED SUCCESS';}catch(e){steps.failClosed=String(e);}
  return steps;
`);
fs.writeFileSync(outDir+"/figure-export.json",JSON.stringify(out,null,2)+"\n");
console.log(JSON.stringify(out,null,2)); s.close();
