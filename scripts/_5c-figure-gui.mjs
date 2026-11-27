import fs from "node:fs"; import { Buffer } from "node:buffer"; import { WebSocket } from "ws";
const port=Number(process.argv[2]); const outDir=process.argv[3]; fs.mkdirSync(outDir,{recursive:true});
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={},t=60000)=>new Promise((r,j)=>{const i=++id;const tm=setTimeout(()=>{p.delete(i);j(new Error(m+" timeout"));},t);p.set(i,{res:v=>{clearTimeout(tm);r(v);},rej:e=>{clearTimeout(tm);j(e);}});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async(e,t=60000)=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true},t);if(r.exceptionDetails)throw new Error(r.exceptionDetails.exception?.description??JSON.stringify(r.exceptionDetails));return r.result?.value;};
const shot=async n=>{const r=await call("Page.captureScreenshot",{format:"png",captureBeyondViewport:true});fs.writeFileSync(`${outDir}/${n}.png`,Buffer.from(r.data,"base64"));};
await call("Runtime.enable"); await call("Page.enable");

// Open project + Methods editor.
const step1=await ev(`
  const wait=async(t,l=20000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,150));}return false;};
  const nav=[...document.querySelectorAll('button,[role=button],a')].find(x=>x.innerText.trim()==='Research Studio'); nav&&nav.click(); await new Promise(r=>setTimeout(r,500));
  const proj=(await window.__TAURI_INTERNALS__.invoke('list_local_research_projects'))[0];
  const b=[...document.querySelectorAll('.rs-project-title')].find(x=>x.innerText.includes('FedAvg')); b&&b.click();
  await wait(()=>Boolean(document.querySelector('.rs-manuscript-workbench')));
  // Find the Methods draft article and click its Edit button.
  const arts=[...document.querySelectorAll('.rs-draft')];
  const methods=arts.find(a=>a.querySelector('header strong')?.innerText.includes('Experimental Setup and Methods'));
  if(!methods) throw new Error('Methods draft not shown');
  const edit=[...methods.querySelectorAll('button')].find(x=>x.innerText.trim()==='Edit');
  edit.click();
  await wait(()=>Boolean(document.querySelector('.rs-draft textarea')));
  return {projectId:proj.id, editorOpen:Boolean(document.querySelector('.rs-draft textarea'))};
`);

// Open the figure picker, confirm preview image, insert the figure.
const step2=await ev(`
  const wait=async(t,l=15000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,150));}return false;};
  const insertBtn=[...document.querySelectorAll('.rs-draft .rs-actions button')].find(x=>x.innerText.trim()==='Insert figure');
  if(!insertBtn) throw new Error('Insert figure button missing');
  insertBtn.click();
  await wait(()=>Boolean(document.querySelector('.rs-figure-picker')));
  await wait(()=>Boolean(document.querySelector('.rs-figure-card img')), 8000);
  const imgOk=Boolean(document.querySelector('.rs-figure-card img')) && document.querySelector('.rs-figure-card img').src.startsWith('data:');
  return {pickerOpen:Boolean(document.querySelector('.rs-figure-picker')), previewImg:imgOk};
`);
await shot("01-figure-picker");

const step3=await ev(`
  const wait=async(t,l=15000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,150));}return false;};
  const ins=[...document.querySelectorAll('.rs-figure-card button')].find(x=>/Insert into section/.test(x.innerText));
  ins.click();
  await wait(()=>Boolean(document.querySelector('.rs-figure-inline')));
  const ta=document.querySelector('.rs-draft textarea');
  const figCount=document.querySelectorAll('.rs-figure-inline li').length;
  const refCount=(ta.value.match(/research-figure:/g)||[]).length;
  const hasFigure1=/Figure 1\./.test(ta.value);
  return {figInlineCount:figCount, refCount, hasFigure1, textTail:ta.value.slice(-240)};
`);
await shot("02-figure-inserted");

// Save as new revision, then approve, then export PDF.
const step4=await ev(`
  const wait=async(t,l=20000)=>{const e=Date.now()+l;while(Date.now()<e){if(t())return true;await new Promise(r=>setTimeout(r,200));}return false;};
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const proj=(await invoke('list_local_research_projects'))[0];
  const save=[...document.querySelectorAll('.rs-draft button')].find(x=>/Save as new revision/.test(x.innerText));
  save.click();
  await wait(()=>!document.querySelector('.rs-draft textarea'));
  const methods=(await invoke('list_local_research_sections',{projectId:proj.id})).filter(x=>x.sectionKey==='methods').sort((a,b)=>b.revision-a.revision)[0];
  // Approve the new figure-bearing revision via its article's Approve button.
  await wait(()=>Boolean([...document.querySelectorAll('.rs-draft')].find(a=>a.querySelector('header strong')?.innerText.includes('Experimental Setup and Methods'))));
  const art=[...document.querySelectorAll('.rs-draft')].find(a=>a.querySelector('header strong')?.innerText.includes('Experimental Setup and Methods'));
  const appr=[...art.querySelectorAll('button')].find(x=>x.innerText.trim()==='Approve section');
  const approvedBefore=methods.status;
  if(appr) appr.click();
  await wait(()=>{const m=document.body.innerText; return true;},1500);
  const after=(await invoke('list_local_research_sections',{projectId:proj.id})).filter(x=>x.sectionKey==='methods');
  const approved=after.find(x=>x.status==='approved');
  const exp=await invoke('export_local_research_section_manuscript',{projectId:proj.id,format:'pdf',allowDraft:false});
  return {newMethodsRev:methods.revision, newMethodsStatus:methods.status, methodsApprovedRev:approved&&approved.revision, export:{pages:exp.actualPages,status:exp.pageTargetStatus,fileSha:exp.fileSha256,artifactPath:exp.artifact.path}};
`, 180000);
await shot("03-after-approve-export");

const out={capturedAtUtc:new Date().toISOString(), step1,step2,step3,step4};
fs.writeFileSync(outDir+"/figure-gui.json",JSON.stringify(out,null,2)+"\n");
console.log(JSON.stringify(out,null,2));
s.close();
