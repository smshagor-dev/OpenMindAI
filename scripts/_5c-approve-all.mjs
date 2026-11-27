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
  const secs=await invoke('list_local_research_sections',{projectId:proj.id});
  const latest={};
  for(const d of secs){ if(!latest[d.sectionKey]||d.revision>latest[d.sectionKey].revision) latest[d.sectionKey]=d; }
  const results=[];
  for(const key of ['introduction','discussion']){
    const d=latest[key];
    if(!d||d.status!=='draft'){results.push({key,skip:d&&d.status});continue;}
    const unsupported=(d.validation&&d.validation.unsupportedTokens)||[];
    if(unsupported.length){results.push({key,blocked:unsupported});continue;}
    try{ await invoke('approve_local_research_section',{draftId:d.id,expectedContentSha256:d.contentSha256}); results.push({key,approved:d.revision, words:d.validation&&d.validation.words, warnings:(d.validation&&d.validation.warnings||[]).length}); }
    catch(e){ results.push({key,error:String(e)}); }
  }
  const approved=(await invoke('list_local_research_sections',{projectId:proj.id})).filter(x=>x.status==='approved').map(x=>x.sectionKey);
  return {results, approvedSections:approved};
`),null,2));
s.close();
