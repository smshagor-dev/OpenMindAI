import fs from "node:fs";
import { WebSocket } from "ws";

const port = Number(process.argv[2]);
const outputPath = process.argv[3];
if (!Number.isInteger(port) || !outputPath) {
  throw new Error("Usage: node probe-native-tauri-bridge.mjs <port> <output.json>");
}

const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const pages = targets.filter((target) => target.type === "page");
if (pages.length !== 1) {
  throw new Error(`Expected exactly one process-owned page; found ${pages.length}`);
}

const page = pages[0];
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.once("open", resolve);
  socket.once("error", reject);
});

const payload = await new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error("CDP evaluation timed out")), 30_000);
  socket.on("message", (raw) => {
    const message = JSON.parse(raw.toString());
    if (message.id !== 1) return;
    clearTimeout(timer);
    if (message.error) reject(new Error(JSON.stringify(message.error)));
    else resolve(message.result?.result?.value ?? message.result);
  });

  const expression = `
    (async () => {
      const internals = window.__TAURI_INTERNALS__;
      const page = {
        href: location.href,
        origin: location.origin,
        title: document.title,
        windowLabel: internals?.metadata?.currentWindow?.label ?? null,
        webviewLabel: internals?.metadata?.currentWebview?.label ?? null,
        internalKeys: Object.keys(internals ?? {}),
        invokeType: typeof internals?.invoke,
        appShell: Boolean(document.querySelector(".app-shell")),
        loader: Boolean(document.querySelector(".app-loading")),
        body: document.body?.innerText?.slice(0, 500) ?? "",
      };
      const commands = [
        { name: "plugin:app|version" },
        { name: "get_portable_root" },
        { name: "list_projects", args: { limit: 10 } },
        { name: "list_local_research_projects" },
        { name: "list_studio3d_projects" },
      ];
      const results = [];
      for (const { name, args } of commands) {
        const started = performance.now();
        try {
          const value = await internals.invoke(name, args);
          results.push({ command: name, ok: true, durationMs: Math.round(performance.now() - started), value });
        } catch (error) {
          results.push({
            command: name,
            ok: false,
            durationMs: Math.round(performance.now() - started),
            error: { name: error?.name ?? null, message: String(error?.message ?? error), stack: error?.stack ?? null },
          });
        }
      }
      return { page, results };
    })()
  `;
  socket.send(JSON.stringify({
    id: 1,
    method: "Runtime.evaluate",
    params: { expression, awaitPromise: true, returnByValue: true },
  }));
});

socket.close();
const result = {
  capturedAt: new Date().toISOString(),
  target: { id: page.id, title: page.title, url: page.url, type: page.type },
  ...payload,
};
fs.writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`);
console.log(JSON.stringify(result, null, 2));

if (
  !payload.page.invokeType ||
  payload.page.windowLabel !== "main" ||
  payload.page.webviewLabel !== "main" ||
  payload.page.origin === "null" ||
  payload.page.href.startsWith("chrome-error:") ||
  payload.page.href.includes(":1420/")
) {
  process.exitCode = 1;
}
const requiredCommands = [
  "get_portable_root",
  "list_projects",
  "list_local_research_projects",
  "list_studio3d_projects",
];
const resultByCommand = new Map(payload.results.map((result) => [result.command, result]));
if (requiredCommands.some((name) => resultByCommand.get(name)?.ok !== true)) {
  process.exitCode = 1;
}
