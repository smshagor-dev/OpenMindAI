import { WebSocket } from "ws";
const port=Number(process.argv[2]); const svg=JSON.stringify(process.argv[3].replaceAll("/","\\"));
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={})=>new Promise((r,j)=>{const i=++id;p.set(i,{res:r,rej:j});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async e=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true});return r;};
await call("Runtime.enable");
const r=await ev(`
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const proj=(await invoke('list_local_research_projects'))[0];
  const steps={};
  try{const fig=await invoke('register_local_research_figure',{projectId:proj.id,sourceRunId:null,path:${svg},caption:'Figure 1. Per-client held-out accuracy.',altText:'Grouped bars per client.'});steps.register='ok:'+fig.id;}catch(e){steps.register='ERR:'+String(e);}
  return steps;
`);
console.log(JSON.stringify(r.exceptionDetails??r.result?.value,null,2)); s.close();
