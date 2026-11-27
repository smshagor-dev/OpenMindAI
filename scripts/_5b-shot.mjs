import fs from "node:fs"; import { Buffer } from "node:buffer"; import { WebSocket } from "ws";
const port=Number(process.argv[2]); const outPng=process.argv[3]; const action=process.argv[4];
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={})=>new Promise((r,j)=>{const i=++id;p.set(i,{res:r,rej:j});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async e=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw new Error(JSON.stringify(r.exceptionDetails));return r.result?.value;};
await call("Runtime.enable"); await call("Page.enable");
const nav=await ev(`
  const wait=async(t,l=15000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,150));}return false;};
  const n=[...document.querySelectorAll('button,[role=button],a')].find(x=>x.innerText.trim()==='Research Studio'); n&&n.click(); await new Promise(r=>setTimeout(r,500));
  const b=[...document.querySelectorAll('.rs-project-title')].find(x=>x.innerText.includes('FedAvg')); b&&b.click();
  await wait(()=>Boolean(document.querySelector('.rs-manuscript-workbench')));
  ${action==='ledger'?`const lb=[...document.querySelectorAll('.rs-ledger button')].find(x=>/claim-to-source ledger/.test(x.innerText)); if(lb&&/Show/.test(lb.innerText))lb.click(); await new Promise(r=>setTimeout(r,400));`:''}
  const dr=document.querySelector('.rs-draft'); if(dr)dr.scrollIntoView({block:'center'}); await new Promise(r=>setTimeout(r,300));
  return {hasDraft:Boolean(document.querySelector('.rs-draft')), figures: await window.__TAURI_INTERNALS__.invoke('list_local_research_figure_assets',{projectId:(await window.__TAURI_INTERNALS__.invoke('list_local_research_projects'))[0].id})};
`);
const r=await call("Page.captureScreenshot",{format:"png",captureBeyondViewport:true});
fs.writeFileSync(outPng,Buffer.from(r.data,"base64"));
console.log(JSON.stringify(nav,null,2)); s.close();
