import { WebSocket } from "ws";
const port=Number(process.argv[2]);
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={},t=30000)=>new Promise((r,j)=>{const i=++id;const tm=setTimeout(()=>{p.delete(i);j(new Error(m+" timeout"));},t);p.set(i,{res:v=>{clearTimeout(tm);r(v);},rej:e=>{clearTimeout(tm);j(e);}});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async(e,t=30000)=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true},t);if(r.exceptionDetails)throw new Error(r.exceptionDetails.exception?.description??JSON.stringify(r.exceptionDetails));return r.result?.value;};
await call("Runtime.enable");
const out=await ev(`
  const wait=async(t,l=15000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,120));}return false;};
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const nav=[...document.querySelectorAll('button,[role=button],a')].find(x=>x.innerText.trim()==='Research Studio'); nav&&nav.click(); await new Promise(r=>setTimeout(r,400));
  const proj=(await invoke('list_local_research_projects'))[0];
  const b=[...document.querySelectorAll('.rs-project-title')].find(x=>x.innerText.includes('FedAvg')); b&&b.click();
  await new Promise(r=>setTimeout(r,500));
  // The Target Pages select lives in the create/edit form.
  const sel=[...document.querySelectorAll('select')].find(x=>[...x.options].some(o=>/24 pages/.test(o.textContent)));
  if(!sel) throw new Error('page target select not found');
  const presets=[...sel.options].map(o=>o.textContent.trim());
  // Set to Custom then type 17.
  const setNative=(el,val,proto)=>{const d=Object.getOwnPropertyDescriptor(proto,'value').set;d.call(el,val);el.dispatchEvent(new Event('change',{bubbles:true}));el.dispatchEvent(new Event('input',{bubbles:true}));};
  setNative(sel,'custom',HTMLSelectElement.prototype);
  await wait(()=>Boolean([...document.querySelectorAll('input[type=number]')].find(x=>/custom page/i.test(x.getAttribute('aria-label')||x.previousSibling?.textContent||''))||document.querySelector('input[type=number]')));
  const custom=[...document.querySelectorAll('input[type=number]')][0];
  setNative(custom,'17',HTMLInputElement.prototype);
  await new Promise(r=>setTimeout(r,200));
  // Save draft.
  const save=[...document.querySelectorAll('.rs-actions button')].find(x=>/Save Draft/.test(x.innerText));
  const saveDisabledAt17 = save?.disabled ?? null;
  save.click();
  await wait(async()=>{const pr=(await invoke('list_local_research_projects'))[0];return pr.manuscript_page_target===17;});
  const after17=(await invoke('list_local_research_projects'))[0].manuscript_page_target;
  // Invalid: 0 and 101 should disable save / show error.
  setNative(custom,'0',HTMLInputElement.prototype); await new Promise(r=>setTimeout(r,150));
  const invalid0 = ([...document.querySelectorAll('.rs-actions button')].find(x=>/Save Draft/.test(x.innerText))?.disabled) || Boolean(document.querySelector('.rs-error,[role=alert]'));
  setNative(custom,'101',HTMLInputElement.prototype); await new Promise(r=>setTimeout(r,150));
  const invalid101 = ([...document.querySelectorAll('.rs-actions button')].find(x=>/Save Draft/.test(x.innerText))?.disabled) || Boolean(document.querySelector('.rs-error,[role=alert]'));
  // Back to preset 24.
  setNative(sel,[...sel.options].find(o=>/24 pages/.test(o.textContent)).value,HTMLSelectElement.prototype);
  await new Promise(r=>setTimeout(r,150));
  const save2=[...document.querySelectorAll('.rs-actions button')].find(x=>/Save Draft/.test(x.innerText)); save2.click();
  await wait(async()=>{const pr=(await invoke('list_local_research_projects'))[0];return pr.manuscript_page_target===24;});
  const after24=(await invoke('list_local_research_projects'))[0].manuscript_page_target;
  return {presets, saveDisabledAt17, after17, invalid0Blocked:invalid0, invalid101Blocked:invalid101, after24};
`);
console.log(JSON.stringify(out,null,2));
s.close();
