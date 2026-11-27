import fs from "node:fs"; import { Buffer } from "node:buffer"; import { WebSocket } from "ws";
const port=Number(process.argv[2]); const outDir=process.argv[3]; fs.mkdirSync(outDir,{recursive:true});
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={},t=60000)=>new Promise((r,j)=>{const i=++id;const tm=setTimeout(()=>{p.delete(i);j(new Error(m+" timeout"));},t);p.set(i,{res:v=>{clearTimeout(tm);r(v);},rej:e=>{clearTimeout(tm);j(e);}});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async(e,t=60000)=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true},t);if(r.exceptionDetails)throw new Error(r.exceptionDetails.exception?.description??JSON.stringify(r.exceptionDetails));return r.result?.value;};
const shot=async n=>{const r=await call("Page.captureScreenshot",{format:"png",captureBeyondViewport:false});fs.writeFileSync(`${outDir}/${n}.png`,Buffer.from(r.data,"base64"));};
await call("Runtime.enable"); await call("Page.enable");
const out=await ev(`
  const wait=async(t,l=15000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,120));}return false;};
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const nav=[...document.querySelectorAll('button,[role=button],a')].find(x=>x.innerText.trim()==='Research Studio'); nav&&nav.click(); await new Promise(r=>setTimeout(r,400));
  const b=[...document.querySelectorAll('.rs-project-title')].find(x=>x.innerText.includes('FedAvg')); b&&b.click();
  await wait(()=>Boolean(document.querySelector('.rs-manuscript-workbench')));
  // Edit the Results draft (latest).
  const arts=[...document.querySelectorAll('.rs-draft')];
  const results=arts.find(a=>a.querySelector('header strong')?.innerText.trim()==='Results');
  [...results.querySelectorAll('button')].find(x=>x.innerText.trim()==='Edit').click();
  await wait(()=>Boolean(document.querySelector('.rs-draft textarea')));
  const ta=document.querySelector('.rs-draft textarea');
  // Move cursor to end, insert figure once.
  ta.focus(); ta.selectionStart=ta.value.length;
  const openPicker=()=>{[...document.querySelectorAll('.rs-draft .rs-actions button')].find(x=>x.innerText.trim()==='Insert figure').click();};
  openPicker(); await wait(()=>Boolean(document.querySelector('.rs-figure-picker')));
  [...document.querySelectorAll('.rs-figure-card button')].find(x=>/Insert into section/.test(x.innerText)).click();
  await wait(()=>document.querySelectorAll('.rs-figure-inline li').length===1);
  // Insert a second time.
  ta.focus(); ta.selectionStart=ta.value.length;
  openPicker(); await wait(()=>Boolean(document.querySelector('.rs-figure-picker')));
  [...document.querySelectorAll('.rs-figure-card button')].find(x=>/Insert into section/.test(x.innerText)).click();
  await wait(()=>document.querySelectorAll('.rs-figure-inline li').length===2);
  const after2=document.querySelector('.rs-draft textarea').value;
  const nums2=(after2.match(/Figure \d+\./g)||[]);
  // Move figure 2 up.
  const moveUp=[...document.querySelectorAll('.rs-figure-inline li')][1].querySelector('button[aria-label^="Move figure 2 up"]');
  moveUp.click(); await new Promise(r=>setTimeout(r,200));
  // Remove figure 1.
  const rm=[...document.querySelectorAll('.rs-figure-inline li')][0].querySelector('button[aria-label^="Remove figure 1"]');
  rm.click(); await wait(()=>document.querySelectorAll('.rs-figure-inline li').length===1);
  const after3=document.querySelector('.rs-draft textarea').value;
  const refsLeft=(after3.match(/research-figure:/g)||[]).length;
  const nums3=(after3.match(/Figure \d+\./g)||[]);
  document.querySelector('.rs-figure-inline')?.scrollIntoView({block:'center'});
  await new Promise(r=>setTimeout(r,300));
  return {insertedTwo:nums2.length, numbering2:nums2, afterMoveRemove_refs:refsLeft, numbering3:nums3};
`);
await shot("04-figure-ops");
fs.writeFileSync(outDir+"/figops.json",JSON.stringify(out,null,2)+"\n");
console.log(JSON.stringify(out,null,2));
s.close();
