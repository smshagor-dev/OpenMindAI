import fs from "node:fs";
import path from "node:path";
import { WebSocket } from "ws";

const port = Number(process.argv[2]);
const outputDir = process.argv[3];
const projectTitle = process.argv[4] ?? "Test 6 evidence import acceptance";
if (!Number.isInteger(port) || !outputDir) throw new Error("Usage: node capture-native-test6-evidence.mjs <debug-port> <output-dir> [project-title]");
fs.mkdirSync(outputDir, { recursive: true });
const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = pages.find((target) => target.type === "page");
if (!page) throw new Error("Native WebView page was not found.");
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.once("open", resolve);
  socket.once("error", reject);
});
let id = 0;
const call = (method, params = {}) => new Promise((resolve, reject) => {
  const requestId = ++id;
  const listener = (raw) => {
    const message = JSON.parse(raw.toString());
    if (message.id !== requestId) return;
    socket.off("message", listener);
    if (message.error) reject(new Error(JSON.stringify(message.error)));
    else resolve(message.result);
  };
  socket.on("message", listener);
  socket.send(JSON.stringify({ id: requestId, method, params }));
});
await call("Runtime.enable");
const evaluated = await call("Runtime.evaluate", {
  expression: `
    (async () => {
      const nav = [...document.querySelectorAll("button,[role=button],a")]
        .find((element) => element.innerText.trim() === "Research Studio");
      nav?.click();
      await new Promise((resolve) => setTimeout(resolve, 700));
      const projectButton = [...document.querySelectorAll("button")]
        .find((element) => element.innerText.includes(${JSON.stringify(projectTitle)}));
      projectButton?.click();
      await new Promise((resolve) => setTimeout(resolve, 900));
      const project = (await window.__TAURI_INTERNALS__.invoke("list_local_research_projects"))
        .find((item) => item.title === ${JSON.stringify(projectTitle)});
      const docs = project
        ? await window.__TAURI_INTERNALS__.invoke("list_local_research_documents", { projectId: project.id })
        : [];
      const uploader = document.querySelector(".rs-evidence-drop");
      uploader?.scrollIntoView({ block: "center" });
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return {
        project,
        docs,
        viewport: { width: innerWidth, height: innerHeight },
        labelsVisible: [...document.querySelectorAll(".rs-local-sources li")].map((item) => item.innerText),
        titleInference: [...document.querySelectorAll(".rs-title-suggestion-blocked")].map((item) => item.innerText),
        modelReadiness: [...document.querySelectorAll(".rs-readiness")].map((item) => item.innerText),
        uploaderText: uploader?.innerText ?? null,
        visibleNotice: [...document.querySelectorAll('[role="status"]')].map((item) => item.innerText).filter(Boolean),
      };
    })()
  `,
  awaitPromise: true,
  returnByValue: true,
});
const value = evaluated.result?.value;
if (!value?.project) throw new Error(`Research project was not found in the native UI: ${projectTitle}`);
const screenshot = await call("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
const screenshotPath = path.join(outputDir, "native-evidence-project.png");
fs.writeFileSync(screenshotPath, Buffer.from(screenshot.data, "base64"));
const report = {
  capturedAtUtc: new Date().toISOString(),
  target: { id: page.id, title: page.title, url: page.url, type: page.type },
  project: value.project,
  documentCount: value.docs.length,
  distinctContentHashes: new Set(value.docs.map((doc) => doc.contentSha256)).size,
  documents: value.docs.map((doc) => ({
    id: doc.id,
    name: doc.name,
    mediaType: doc.mediaType,
    sizeBytes: doc.sizeBytes,
    contentSha256: doc.contentSha256,
    extractionMetadata: JSON.parse(doc.extractionMetadataJson),
  })),
  ui: {
    viewport: value.viewport,
    labelsVisible: value.labelsVisible,
    titleInference: value.titleInference,
    modelReadiness: value.modelReadiness,
    uploaderText: value.uploaderText,
    visibleNotice: value.visibleNotice,
  },
  screenshot: screenshotPath,
};
const reportPath = path.join(outputDir, "native-evidence-project.json");
fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify({ reportPath, screenshotPath, project: value.project, documentCount: value.docs.length, distinctContentHashes: report.distinctContentHashes }, null, 2));
socket.close();
