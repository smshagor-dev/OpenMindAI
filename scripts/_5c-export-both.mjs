import fs from "node:fs"; import { WebSocket } from "ws";
const port=Number(process.argv[2]); const outDir=process.argv[3]; fs.mkdirSync(outDir,{recursive:true});
const pages=(await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter(t=>t.type==="page");
const s=new WebSocket(pages[0].webSocketDebuggerUrl); await new Promise((r,j)=>{s.once("open",r);s.once("error",j);});
let id=0;const p=new Map();s.on("message",raw=>{const m=JSON.parse(raw);const w=p.get(m.id);if(!w)return;p.delete(m.id);m.error?w.rej(new Error(JSON.stringify(m.error))):w.res(m.result);});
const call=(m,pr={},t=180000)=>new Promise((r,j)=>{const i=++id;const tm=setTimeout(()=>{p.delete(i);j(new Error(m+" timeout"));},t);p.set(i,{res:v=>{clearTimeout(tm);r(v);},rej:e=>{clearTimeout(tm);j(e);}});s.send(JSON.stringify({id:i,method:m,params:pr}));});
const ev=async(e,t=180000)=>{const r=await call("Runtime.evaluate",{expression:`(async()=>{${e}})()`,awaitPromise:true,returnByValue:true},t);if(r.exceptionDetails)throw new Error(JSON.stringify(r.exceptionDetails.exception??r.exceptionDetails));return r.result?.value;};
await call("Runtime.enable");
const out=await ev(`
  const invoke=window.__TAURI_INTERNALS__.invoke;
  const proj=(await invoke('list_local_research_projects'))[0];
  const pdf1=await invoke('export_local_research_section_manuscript',{projectId:proj.id,format:'pdf',allowDraft:false});
  const docx1=await invoke('export_local_research_section_manuscript',{projectId:proj.id,format:'docx',allowDraft:false});
  const pdf2=await invoke('export_local_research_section_manuscript',{projectId:proj.id,format:'pdf',allowDraft:false});
  const docx2=await invoke('export_local_research_section_manuscript',{projectId:proj.id,format:'docx',allowDraft:false});
  const pick=e=>({pages:e.actualPages,status:e.pageTargetStatus,words:e.wordCount,fileSha:e.fileSha256,contentSha:e.contentSha256,ledgerSha:e.ledgerSha256,path:e.artifact.path});
  return {pdf1:pick(pdf1),docx1:pick(docx1),pdf2:pick(pdf2),docx2:pick(docx2),
    pdfContentStable:pdf1.contentSha256===pdf2.contentSha256, pdfFileStable:pdf1.fileSha256===pdf2.fileSha256,
    docxContentStable:docx1.contentSha256===docx2.contentSha256};
`);
fs.writeFileSync(outDir+"/export-both.json",JSON.stringify(out,null,2)+"\n");
console.log(JSON.stringify(out,null,2));
s.close();
