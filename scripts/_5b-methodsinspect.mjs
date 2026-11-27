import fs from "node:fs"; import { Buffer } from "node:buffer"; import { WebSocket } from "ws";
const port=Number(process.argv[2]); const outDir=process.argv[3];
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={})=>new Promise((r,j)=>{const i=++id;p.set(i,{res:r,rej:j});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async e=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw new Error(JSON.stringify(r.exceptionDetails));return r.result?.value;};
await call("Runtime.enable");
const out=await ev(`
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const proj=(await invoke('list_local_research_projects'))[0];
  const m=(await invoke('list_local_research_sections',{projectId:proj.id})).filter(x=>x.sectionKey==='methods').sort((a,b)=>b.revision-a.revision)[0];
  const ledger=(await invoke('local_research_claim_ledger',{projectId:proj.id})).filter(l=>l.sectionDraftId===m.id);
  return {status:m.status,rev:m.revision,model:m.modelId,latencyMs:m.latencyMs,attempts:m.attempts,words:m.validation&&m.validation.words,unsupported:m.validation&&m.validation.unsupportedTokens,warnings:m.validation&&m.validation.warnings,ledgerCount:ledger.length,content:m.content};
`);
fs.writeFileSync(outDir+"/methods.json",JSON.stringify(out,null,2)+"\n");
console.log("status:",out.status,"rev:",out.rev,"latency:",out.latencyMs,"attempts:",out.attempts,"words:",out.words,"ledger:",out.ledgerCount,"unsupported:",JSON.stringify(out.unsupported));
console.log("--- content ---"); console.log(out.content);
s.close();
