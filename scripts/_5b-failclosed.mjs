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
  // The latest data_availability approved revision is the tampered one; export must fail closed.
  let msg=null,code=null;
  try{await invoke('export_local_research_section_manuscript',{projectId:proj.id,format:'pdf',allowDraft:false});msg='UNEXPECTED SUCCESS';}
  catch(e){msg=(e&&e.message)||JSON.stringify(e);code=e&&e.code;}
  return {failClosedMessage:msg,code};
`),null,2));
s.close();
