/* global fetch */
import fs from "node:fs";
import { WebSocket } from "ws";

const [portArg, outputPath] = process.argv.slice(2);
const port = Number(portArg);
if (!Number.isInteger(port) || !outputPath) throw new Error("Usage: node accept-native-optional-title.mjs <process-owned-webview-port> <output.json>");
const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = pages.find((target) => target.type === "page");
if (!page || pages.filter((target) => target.type === "page").length !== 1) throw new Error("Expected one process-owned native WebView.");
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.once("open", resolve); socket.once("error", reject); });
let id = 0;
const call = (method, params = {}) => new Promise((resolve, reject) => {
  const requestId = ++id;
  const listener = (raw) => { const message = JSON.parse(raw.toString()); if (message.id !== requestId) return; socket.off("message", listener); message.error ? reject(new Error(JSON.stringify(message.error))) : resolve(message.result); };
  socket.on("message", listener); socket.send(JSON.stringify({ id: requestId, method, params }));
});
await call("Runtime.enable");
const expression = `(
  async () => {
    const nav = [...document.querySelectorAll('button,[role=button],a')].find((item) => item.innerText.trim() === 'Research Studio');
    nav?.click(); await new Promise((resolve) => setTimeout(resolve, 600));
    const fresh = [...document.querySelectorAll('button')].find((item) => item.innerText.trim() === 'New local research');
    fresh?.click(); await new Promise((resolve) => setTimeout(resolve, 300));
    const prompt = document.querySelector('textarea[aria-label="Research Prompt (Required)"]');
    const title = document.querySelector('input[aria-label="Research Title (Optional)"]');
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
    setter.call(prompt, 'Disposable acceptance check: optional blank title persistence.');
    prompt.dispatchEvent(new Event('input', { bubbles: true }));
    if (title) { const titleSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set; titleSetter.call(title, ''); title.dispatchEvent(new Event('input', { bubbles: true })); }
    const save = [...document.querySelectorAll('.rs-actions button')].find((item) => item.innerText.includes('Save Draft'));
    const generate = [...document.querySelectorAll('.rs-actions button')].find((item) => item.innerText.includes('Generate Research'));
    const blocked = document.querySelector('.rs-title-suggestion-blocked')?.innerText ?? null;
    save?.click(); await new Promise((resolve) => setTimeout(resolve, 700));
    const projects = await window.__TAURI_INTERNALS__.invoke('list_local_research_projects');
    const saved = projects.find((item) => item.title === 'Untitled research' && item.prompt === 'Disposable acceptance check: optional blank title persistence.');
    return { savedProject: saved ?? null, blankTitle: title?.value === '', titleNotice: blocked, saveEnabled: Boolean(save && !save.disabled), generateDisabledWithoutModel: Boolean(generate?.disabled), modelReadiness: document.querySelector('.rs-readiness')?.innerText ?? null, appNotice: [...document.querySelectorAll('[role=status]')].map((item) => item.innerText).find((value) => value.includes('Research draft')) ?? null };
  }
)()`;
const evaluated = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
const result = evaluated.result?.value;
if (!result) throw new Error(JSON.stringify(evaluated));
const output = { capturedAtUtc: new Date().toISOString(), processOwnedPage: { id: page.id, url: page.url }, result };
fs.writeFileSync(outputPath, `${JSON.stringify(output, null, 2)}\n`);
console.log(JSON.stringify(output, null, 2));
socket.close();
