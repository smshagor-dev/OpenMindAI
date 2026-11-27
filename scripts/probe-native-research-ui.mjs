import fs from "node:fs";
import path from "node:path";
import { WebSocket } from "ws";

const port = Number(process.argv[2]);
const outputDir = process.argv[3];
const requestedWidth = process.argv[4] ? Number(process.argv[4]) : null;
const requestedHeight = process.argv[5] ? Number(process.argv[5]) : null;
if (!Number.isInteger(port) || !outputDir || (requestedWidth && !requestedHeight)) {
  throw new Error("Usage: node probe-native-research-ui.mjs <port> <output-dir> [width height]");
}
fs.mkdirSync(outputDir, { recursive: true });

const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const pages = targets.filter((target) => target.type === "page");
if (pages.length !== 1) throw new Error(`Expected one native page, found ${pages.length}`);

const page = pages[0];
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.once("open", resolve);
  socket.once("error", reject);
});
let nextId = 1;
const pending = new Map();
socket.on("message", (raw) => {
  const message = JSON.parse(raw.toString());
  const waiter = pending.get(message.id);
  if (!waiter) return;
  pending.delete(message.id);
  if (message.error) waiter.reject(new Error(JSON.stringify(message.error)));
  else waiter.resolve(message.result);
});
const send = (method, params = {}) => new Promise((resolve, reject) => {
  const id = nextId++;
  pending.set(id, { resolve, reject });
  socket.send(JSON.stringify({ id, method, params }));
});

if (requestedWidth && requestedHeight) {
  const { windowId } = await send("Browser.getWindowForTarget", { targetId: page.id });
  await send("Browser.setWindowBounds", { windowId, bounds: { width: requestedWidth, height: requestedHeight, windowState: "normal" } });
  await new Promise((resolve) => setTimeout(resolve, 500));
}
await send("Runtime.enable");
const ui = await send("Runtime.evaluate", {
  expression: `
    (async () => {
      const nav = [...document.querySelectorAll("button,[role=button],a")]
        .find((item) => item.innerText.trim() === "Research Studio");
      if (!nav) throw new Error("Research Studio navigation control was not found");
      nav.click();
      await new Promise((resolve) => setTimeout(resolve, 900));
      const controls = [...document.querySelectorAll("input,textarea,select,button")]
        .filter((item) => item.getClientRects().length)
        .map((item) => ({
          tag: item.tagName.toLowerCase(),
          type: item.type ?? null,
          name: item.name ?? null,
          id: item.id ?? null,
          value: item.value ?? null,
          text: (item.innerText ?? item.getAttribute("aria-label") ?? item.placeholder ?? "").trim().slice(0, 120),
          disabled: Boolean(item.disabled),
          bounds: (() => { const r = item.getBoundingClientRect(); return { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height), bottom: Math.round(r.bottom) }; })(),
        }));
      const body = document.body.innerText;
      const report = {
        href: location.href,
        title: document.title,
        windowLabel: window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label,
        viewport: { width: innerWidth, height: innerHeight, scrollWidth: document.documentElement.scrollWidth, scrollHeight: document.documentElement.scrollHeight },
        body,
        controls,
        evidenceHeadingIndex: body.indexOf("Evidence"),
        titleIndex: body.indexOf("Research Title"),
        promptIndex: body.indexOf("Research Prompt"),
        pagesIndex: body.indexOf("Target Pages"),
        chooserAvailable: controls.some((item) => item.type === "file"),
        inputTypes: [...document.querySelectorAll("input[type=file]")].map((item) => ({ accept: item.accept, multiple: item.multiple })),
      };
      const savedProjectButton = [...document.querySelectorAll("button")].find((item) => item.innerText.includes("Test 6 acceptance draft"));
      if (savedProjectButton) {
        savedProjectButton.click();
        await new Promise((resolve) => setTimeout(resolve, 500));
        const pageSelect = [...document.querySelectorAll("select")].find((item) => [...item.options].some((option) => option.textContent.includes("24 pages")));
        report.reloadedProject = {
          title: document.querySelector('input[aria-label="Research Title (Optional)"]')?.value ?? null,
          prompt: document.querySelector('textarea[aria-label="Research Prompt (Required)"]')?.value ?? null,
          pageValue: pageSelect?.value ?? null,
          pageText: pageSelect?.selectedOptions?.[0]?.textContent ?? null,
        };
      }
      const scrollport = document.querySelector(".rs.rs-main");
      if (scrollport) {
        scrollport.scrollTop = scrollport.scrollHeight;
        await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        const footer = document.querySelector(".workspace-footer");
        const generate = [...document.querySelectorAll(".rs-actions button")].find((item) => item.innerText.includes("Generate Research"));
        const bounds = (item) => {
          if (!item) return null;
          const rect = item.getBoundingClientRect();
          return { top: Math.round(rect.top), bottom: Math.round(rect.bottom), height: Math.round(rect.height) };
        };
        report.bottomLayout = {
          scrollTop: Math.round(scrollport.scrollTop),
          scrollHeight: Math.round(scrollport.scrollHeight),
          clientHeight: Math.round(scrollport.clientHeight),
          footer: bounds(footer),
          generate: bounds(generate),
          generateAboveFooter: Boolean(generate && footer && generate.getBoundingClientRect().bottom <= footer.getBoundingClientRect().top),
        };
      }
      return report;
    })()
  `,
  awaitPromise: true,
  returnByValue: true,
});
const value = ui.result?.value;
if (!value) throw new Error("Native research UI returned no DOM data");

const screenshot = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
const screenshotPath = path.join(outputDir, "native-research-create.png");
fs.writeFileSync(screenshotPath, Buffer.from(screenshot.data, "base64"));
const report = {
  capturedAt: new Date().toISOString(),
  target: { id: page.id, title: page.title, url: page.url, type: page.type },
  ui: value,
  screenshot: screenshotPath,
};
const jsonPath = path.join(outputDir, "native-research-ui.json");
fs.writeFileSync(jsonPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify(report, null, 2));
socket.close();

if (value.titleIndex < 0 || value.promptIndex < 0 || value.pagesIndex < 0 || value.evidenceHeadingIndex < 0) {
  process.exitCode = 1;
}
