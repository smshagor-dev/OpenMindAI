import fs from "node:fs"; import { WebSocket } from "ws";
const port=Number(process.argv[2]); const outDir=process.argv[3];
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={})=>new Promise((r,j)=>{const i=++id;p.set(i,{res:r,rej:j});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async e=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw new Error(JSON.stringify(r.exceptionDetails.exception??r.exceptionDetails));return r.result?.value;};
await call("Runtime.enable");
const out=await ev(`
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const bridge={};
  for(const [name,args] of [['get_portable_root',{}],['list_projects',{}],['list_local_research_projects',{}],['list_studio3d_projects',{}]]){
    try{ const r=await invoke(name,args); bridge[name]=Array.isArray(r)?('ok['+r.length+']'):'ok'; }catch(e){ bridge[name]='ERR:'+String(e); }
  }
  const proj=(await invoke('list_local_research_projects'))[0];
  const secs=await invoke('list_local_research_sections',{projectId:proj.id});
  const ledger=await invoke('local_research_claim_ledger',{projectId:proj.id});
  const figs=await invoke('list_local_research_figure_assets',{projectId:proj.id});
  const approved=secs.filter(x=>x.status==='approved');
  return {bridge, pageTarget:proj.manuscript_page_target, totalSectionRevisions:secs.length,
    approvedSections:approved.map(x=>x.sectionKey+':r'+x.revision),
    ledgerRows:ledger.length, figures:figs.length,
    lifecycle:['results','methods','discussion','introduction','data_availability'].map(k=>({k, revs:secs.filter(x=>x.sectionKey===k).map(x=>x.status)}))};
`);
if(outDir) fs.writeFileSync(outDir+"/final-verify.json",JSON.stringify(out,null,2)+"\n");
console.log(JSON.stringify(out,null,2));
s.close();
