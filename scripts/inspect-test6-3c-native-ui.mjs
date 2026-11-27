import { WebSocket } from "ws";

const port = Number(process.argv[2] ?? 9457);
const pages = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter((target) => target.type === "page");
if (pages.length !== 1) throw new Error(`Expected one native app page, received ${pages.length}.`);
const socket = new WebSocket(pages[0].webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.once("open", resolve); socket.once("error", reject); });
socket.on("message", (raw) => {
  const message = JSON.parse(raw.toString());
  if (message.id !== 1) return;
  console.log(JSON.stringify(message.result?.result?.value, null, 2));
  socket.close();
});
socket.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression: "({url:location.href, body:document.body.innerText, buttons:[...document.querySelectorAll('button,a,[role=button]')].map(n=>({text:n.innerText.trim(),title:n.title,aria:n.getAttribute('aria-label')})).filter(n=>n.text||n.title||n.aria)})", returnByValue: true } }));
