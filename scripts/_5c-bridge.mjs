import { WebSocket } from "ws";
const port=Number(process.argv[2]);
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={})=>new Promise((r,j)=>{const i=++id;p.set(i,{res:r,rej:j});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async e=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw new Error(JSON.stringify(r.exceptionDetails.exception??r.exceptionDetails));return r.result?.value;};
await call("Runtime.enable");
console.log(JSON.stringify(await ev(`
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const proj=(await invoke('list_local_research_projects'))[0];
  const figs=await invoke('list_local_research_figure_assets',{projectId:proj.id});
  let previewOk=false, previewLen=0, forgedRejected=false, wrongProjectRejected=false;
  if(figs.length){ const url=await invoke('local_research_figure_data_url',{projectId:proj.id,assetId:figs[0].id}); previewOk=url.startsWith('data:'); previewLen=url.length; }
  try{ await invoke('local_research_figure_data_url',{projectId:proj.id,assetId:'00000000-0000-0000-0000-000000000000'}); }catch(e){ forgedRejected=true; }
  try{ await invoke('local_research_figure_data_url',{projectId:proj.id,assetId:'../etc/passwd'}); }catch(e){ wrongProjectRejected=true; }
  return {projectId:proj.id, figureCount:figs.length, figures:figs.map(f=>({id:f.id,name:f.name})), previewOk, previewLen, forgedRejected, traversalRejected:wrongProjectRejected};
`),null,2));
s.close();
