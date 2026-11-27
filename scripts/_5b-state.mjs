import { WebSocket } from "ws";
const port=Number(process.argv[2]);
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={})=>new Promise((r,j)=>{const i=++id;p.set(i,{res:r,rej:j});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async e=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw new Error(JSON.stringify(r.exceptionDetails));return r.result?.value;};
await call("Runtime.enable");
console.log(JSON.stringify(await ev(`
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const proj=(await invoke('list_local_research_projects'))[0];
  const sections=await invoke('list_local_research_sections',{projectId:proj.id});
  const ledger=await invoke('local_research_claim_ledger',{projectId:proj.id});
  return {sections:sections.map(x=>({key:x.sectionKey,rev:x.revision,status:x.status,origin:x.origin,model:x.modelId,latencyMs:x.latencyMs,attempts:x.attempts,err:x.error,words:x.validation&&x.validation.words})),ledgerCount:ledger.length};
`),null,2));
s.close();
