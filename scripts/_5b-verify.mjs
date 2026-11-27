import fs from "node:fs"; import { WebSocket } from "ws";
const port=Number(process.argv[2]); const outDir=process.argv[3]; fs.mkdirSync(outDir,{recursive:true});
const FIG="ea26ad96-90ae-452c-923e-e0c0d01fd437";
const goodContent=`## Data and Code Availability\n\nThe evidence file table-1-heldout-metrics.csv was used; its SHA-256 is recorded in the claim ledger.\n\nThe figure below shows per-client held-out accuracy.\n\n![Grouped bars of per-client held-out accuracy for the two methods.](research-figure:${FIG} "Figure 1. Per-client held-out accuracy for FedAvg and personalized Per-FedAvg (one MNIST IID seed).")`;
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={},t=180000)=>new Promise((r,j)=>{const i=++id;const tm=setTimeout(()=>{p.delete(i);j(new Error(m+" timeout"));},t);p.set(i,{res:v=>{clearTimeout(tm);r(v);},rej:e=>{clearTimeout(tm);j(e);}});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async(e,t=180000)=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true},t);if(r.exceptionDetails)throw new Error(JSON.stringify(r.exceptionDetails.exception??r.exceptionDetails));return r.result?.value;};
await call("Runtime.enable");
const out=await ev(`
  const wait=async(t,l=20000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,150));}return false;};
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const nav=[...document.querySelectorAll('button,[role=button],a')].find(x=>x.innerText.trim()==='Research Studio'); nav&&nav.click(); await new Promise(r=>setTimeout(r,500));
  const proj=(await invoke('list_local_research_projects'))[0];
  const b=[...document.querySelectorAll('.rs-project-title')].find(x=>x.innerText.includes('FedAvg')); b&&b.click();
  await wait(()=>Boolean(document.querySelector('.rs-manuscript-workbench')));
  // Persisted-across-restart state (DB was opened fresh by this process).
  const sections=await invoke('list_local_research_sections',{projectId:proj.id});
  const ledger=await invoke('local_research_claim_ledger',{projectId:proj.id});
  const figures=await invoke('list_local_research_figure_assets',{projectId:proj.id});
  // Re-link a valid figure revision (supersedes the tampered one) so export succeeds.
  const edit=await invoke('save_local_research_section_edit',{projectId:proj.id,sectionKey:'data_availability',baseDraftId:null,content:${JSON.stringify(goodContent)}});
  await invoke('approve_local_research_section',{draftId:edit.id,expectedContentSha256:edit.contentSha256});
  const reExport=await invoke('export_local_research_section_manuscript',{projectId:proj.id,format:'pdf',allowDraft:false});
  const hasRefreshBtn=[...document.querySelectorAll('.rs-evidence-classes button')].some(x=>/Refresh evidence/.test(x.innerText));
  return {
    pageTarget: proj.manuscript_page_target,
    sectionsPersisted: sections.map(x=>({key:x.sectionKey,rev:x.revision,status:x.status,origin:x.origin})),
    ledgerCountPersisted: ledger.length,
    figuresPersisted: figures.map(f=>({id:f.id,name:f.name,sha:f.contentSha256})),
    reExport: {pages:reExport.actualPages,status:reExport.pageTargetStatus,fileSha:reExport.fileSha256,contentSha:reExport.contentSha256,ledgerSha:reExport.ledgerSha256,artifactPath:reExport.artifact.path},
    hasRefreshButton: hasRefreshBtn
  };
`);
fs.writeFileSync(outDir+"/restart-verify.json",JSON.stringify(out,null,2)+"\n");
console.log(JSON.stringify(out,null,2)); s.close();
