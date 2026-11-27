import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { WebSocket } from "ws";

const [portText, fixtureFile, initialFile, outputFile] = process.argv.slice(2);
const port = Number(portText);
if (!Number.isInteger(port) || !fixtureFile || !initialFile || !outputFile) throw new Error("Usage: node verify-test6-3c-native-restart.mjs <port> <fixture.json> <first-run.json> <output.json>");
const fixture = JSON.parse(fs.readFileSync(fixtureFile, "utf8"));
const initial = JSON.parse(fs.readFileSync(initialFile, "utf8"));
const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = targets.find((target) => target.type === "page");
if (!page || targets.filter((target) => target.type === "page").length !== 1) throw new Error("Expected exactly one restarted desktop WebView.");
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.once("open", resolve); socket.once("error", reject); });
let id = 0;
const cdp = (method, params = {}) => new Promise((resolve, reject) => { const requestId = ++id; const listener = (raw) => { const message = JSON.parse(raw.toString()); if (message.id !== requestId) return; socket.off("message", listener); message.error ? reject(new Error(JSON.stringify(message.error))) : resolve(message.result); }; socket.on("message", listener); socket.send(JSON.stringify({ id: requestId, method, params })); });
await cdp("Runtime.enable");
const evaluate = async (expression) => { const result = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }); if (result.exceptionDetails) throw new Error(result.exceptionDetails.text ?? JSON.stringify(result.exceptionDetails)); return result.result?.value; };
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const persisted = await evaluate(`(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const root = await invoke("get_portable_root");
  const projects = await invoke("list_local_research_projects");
  const runs = await invoke("list_local_research_runs", { projectId: ${JSON.stringify(fixture.projectId)} });
  const figures = await invoke("list_local_research_figure_assets", { projectId: ${JSON.stringify(fixture.projectId)} });
  const detail = await invoke("local_research_run_details", { runId: ${JSON.stringify(fixture.runId)} });
  const artifacts = await invoke("list_artifacts", { conversationId: ${JSON.stringify(fixture.conversationId)} });
  return { root, projects, runs, figures, detail, artifacts };
})()`);
if (persisted.root.root !== fixture.root) throw new Error(`Restart opened an unexpected data root: ${persisted.root.root}`);
if (!persisted.projects.some((item) => item.id === fixture.projectId)) throw new Error("Disposable Research Studio project did not survive restart.");
const asset = persisted.figures.find((item) => item.id === initial.registration.id);
if (!asset || asset.contentSha256 !== initial.registration.contentSha256 || asset.projectId !== fixture.projectId) throw new Error("The registered figure ID, project ownership, or source digest changed across restart.");
if (!persisted.detail.manuscript?.content.includes(initial.canonicalReference)) throw new Error("The canonical figure reference did not survive restart.");
const originalArtifacts = initial.exports.map((item) => {
  const row = persisted.artifacts.find((candidate) => candidate.id === item.id);
  const fullPath = path.join(persisted.root.root, item.path);
  const bytes = fs.readFileSync(fullPath);
  const digest = crypto.createHash("sha256").update(bytes).digest("hex");
  if (!row || digest !== item.fileSha256) throw new Error(`Original export ${item.id} was not retained intact after restart.`);
  return { id: item.id, format: item.kind, exists: true, fileSha256: digest, sizeBytes: bytes.length, listedAfterRestart: true };
});
const selected = await evaluate(`(async () => {
  const click = async (selector, text) => { const end = Date.now()+30000; while(Date.now()<end){const el=[...document.querySelectorAll(selector)].find(n=>n.innerText.trim()===text||n.innerText.includes(text));if(el){el.click();return;}await new Promise(r=>setTimeout(r,150));}throw new Error("Missing UI element: "+text);};
  await click("button,a,[role=button]", "Research Studio");
  await click("button.rs-project-title", ${JSON.stringify(fixture.title)});
  await click("button.rs-project-title", "completed");
  return true;
})()`);
const reexports = await evaluate(`(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const known = new Set((await invoke("list_artifacts", { conversationId: ${JSON.stringify(fixture.conversationId)} })).map(item=>item.id));
  const make = async label => { const button=[...document.querySelectorAll("button")].find(n=>n.innerText.trim()===label); if(!button)throw new Error("Missing "+label); button.click(); const end=Date.now()+120000; while(Date.now()<end){ const rows=await invoke("list_artifacts",{conversationId:${JSON.stringify(fixture.conversationId)}}); const found=rows.find(r=>!known.has(r.id)&&r.status==="ready"&&r.kind===(label.includes("PDF")?"pdf":"docx")); if(found){known.add(found.id);return found;} const error=[...document.querySelectorAll('[role="alert"]')].map(n=>n.innerText).filter(Boolean).join("\\n"); if(error)throw new Error(error); await new Promise(r=>setTimeout(r,250)); } throw new Error("Timed out: "+label); };
  return [await make("Export draft PDF"),await make("Export draft DOCX")];
})()`);
const screenshot = async (filename) => { const result=await cdp("Page.captureScreenshot",{format:"png",captureBeyondViewport:false}); const target=path.join(path.dirname(outputFile),filename); fs.writeFileSync(target,Buffer.from(result.data,"base64")); return target; };
const successScreenshot = await screenshot("native-figure-reopened.png");
const root = persisted.root.root;
const reexportsWithHashes = reexports.map((item) => { const fullPath=path.join(root,item.path); const bytes=fs.readFileSync(fullPath); return { ...item, fileSha256:crypto.createHash("sha256").update(bytes).digest("hex"), actualSizeBytes:bytes.length }; });
const failed = await evaluate(`(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const before = await invoke("list_artifacts", { conversationId: ${JSON.stringify(fixture.conversationId)} });
  const known = new Set(before.map(item=>item.id));
  const click = async (selector,text) => { const end=Date.now()+30000; while(Date.now()<end){const el=[...document.querySelectorAll(selector)].find(n=>n.innerText.trim()===text);if(el){el.click();return;}await new Promise(r=>setTimeout(r,150));}throw new Error("Missing "+text);};
  await click("button", "Edit manuscript");
  const editor=document.querySelector('textarea[aria-label="Edit manuscript revision"]');
  if(!editor)throw new Error("Manuscript editor did not open.");
  const setter=Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value").set;
  setter.call(editor,editor.value.trimEnd()+"\\n\\n![Missing test figure](research-figure:00000000-0000-4000-8000-000000000000 \\\"Missing test figure\\\")\\n");
  editor.dispatchEvent(new Event("input",{bubbles:true}));
  await click("button","Save as new revision");
  const waitForSaved=Date.now()+30000; while(Date.now()<waitForSaved){if(!document.querySelector('textarea[aria-label="Edit manuscript revision"]'))break; await new Promise(r=>setTimeout(r,150));}
  const pdfButton=[...document.querySelectorAll("button")].find(n=>n.innerText.trim()==="Export draft PDF"); if(!pdfButton)throw new Error("Missing draft PDF button."); pdfButton.click();
  let pdf={format:"pdf",unexpectedSuccess:true,error:null}; const pdfEnd=Date.now()+30000; while(Date.now()<pdfEnd){const alert=[...document.querySelectorAll('[role="alert"]')].map(n=>n.innerText).filter(Boolean).join("\\n");const rows=await invoke("list_artifacts",{conversationId:${JSON.stringify(fixture.conversationId)}});if(alert){pdf={format:"pdf",unexpectedSuccess:false,error:alert};break;}if(rows.some(r=>!known.has(r.id)&&r.status==="ready"))break;await new Promise(r=>setTimeout(r,150));}
  let docx; try { const value=await invoke("export_local_research_document",{runId:${JSON.stringify(fixture.runId)},format:"docx",allowDraft:true}); docx={format:"docx",unexpectedSuccess:true,value}; } catch(error) { docx={format:"docx",unexpectedSuccess:false,error:String(error)}; }
  const rows=await invoke("list_artifacts",{conversationId:${JSON.stringify(fixture.conversationId)}});
  return {pdf,docx,artifactCountBefore:before.length,artifactCountAfter:rows.length,newArtifactIds:rows.filter(r=>!known.has(r.id)).map(r=>r.id)};
})()`);
if (failed.pdf.unexpectedSuccess || failed.docx.unexpectedSuccess || failed.newArtifactIds.length) throw new Error(`Unresolved figure unexpectedly published an export: ${JSON.stringify(failed)}`);
const failureScreenshot = await screenshot("native-figure-unresolved-failure.png");
const report = { capturedAtUtc: new Date().toISOString(), restartedWebView: { pageId: page.id, url: page.url }, projectPersisted: true, runId: fixture.runId, registeredFigurePersisted: { id: asset.id, projectId: asset.projectId, sourceSha256: asset.contentSha256, widthPx: asset.widthPx, heightPx: asset.heightPx, caption: asset.caption, altText: asset.altText }, canonicalReferencePersisted: persisted.detail.manuscript.content.includes(initial.canonicalReference), originalExportsAfterRestart: originalArtifacts, reexports: reexportsWithHashes, unresolvedReferenceFailClosed: failed, screenshots: { success: successScreenshot, failure: failureScreenshot } };
fs.writeFileSync(outputFile, `${JSON.stringify(report,null,2)}\n`);
console.log(JSON.stringify(report,null,2));
socket.close();
