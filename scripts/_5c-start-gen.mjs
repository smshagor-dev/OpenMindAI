import { WebSocket } from "ws";
const port=Number(process.argv[2]); const sectionKey=process.argv[3]||'discussion';
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={},t=60000)=>new Promise((r,j)=>{const i=++id;const tm=setTimeout(()=>{p.delete(i);j(new Error(m+" timeout"));},t);p.set(i,{res:v=>{clearTimeout(tm);r(v);},rej:e=>{clearTimeout(tm);j(e);}});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async(e,t=60000)=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true},t);if(r.exceptionDetails)throw new Error(r.exceptionDetails.exception?.description??JSON.stringify(r.exceptionDetails));return r.result?.value;};
await call("Runtime.enable");
const heading={discussion:'Discussion',introduction:'Introduction and Gap',abstract:'Abstract'}[sectionKey];
const out=await ev(`
  const wait=async(t,l=20000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,120));}return false;};
  const invoke=window.__TAURI_INTERNALS__.invoke;
  // Cancel any open editor.
  const cancel=[...document.querySelectorAll('.rs-draft button')].find(x=>x.innerText.trim()==='Cancel'); if(cancel) cancel.click();
  const nav=[...document.querySelectorAll('button,[role=button],a')].find(x=>x.innerText.trim()==='Research Studio'); nav&&nav.click(); await new Promise(r=>setTimeout(r,400));
  const proj=(await invoke('list_local_research_projects'))[0];
  const b=[...document.querySelectorAll('.rs-project-title')].find(x=>x.innerText.includes('FedAvg')); b&&b.click();
  await wait(()=>Boolean(document.querySelector('.rs-manuscript-workbench')));
  const before=(await invoke('list_local_research_sections',{projectId:proj.id})).filter(x=>x.sectionKey===${JSON.stringify(sectionKey)}).length;
  const row=[...document.querySelectorAll('.rs-section-table tbody tr')].find(tr=>tr.firstElementChild&&tr.firstElementChild.innerText.trim()===${JSON.stringify(heading)});
  if(!row) throw new Error('section row not found: '+${JSON.stringify(heading)});
  const gen=[...row.querySelectorAll('button')].find(x=>/Generate|Regenerate/.test(x.innerText));
  if(!gen||gen.disabled) throw new Error('Generate disabled');
  gen.click();
  // Confirm the job entered a durable running state.
  const running=await wait(async()=>{const secs=await invoke('list_local_research_sections',{projectId:proj.id});return secs.some(x=>x.sectionKey===${JSON.stringify(sectionKey)}&&x.status==='running');},25000);
  const secs=await invoke('list_local_research_sections',{projectId:proj.id});
  const job=secs.filter(x=>x.sectionKey===${JSON.stringify(sectionKey)}).sort((a,b)=>b.revision-a.revision)[0];
  return {running, sectionKey:${JSON.stringify(sectionKey)}, draftId:job&&job.id, status:job&&job.status, revision:job&&job.revision, before};
`, 40000);
console.log(JSON.stringify(out,null,2));
s.close();
