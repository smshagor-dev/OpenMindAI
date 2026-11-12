import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import { URL, fileURLToPath, pathToFileURL } from "node:url";
import { build } from "esbuild";
import { JSDOM } from "jsdom";
import React from "react";

const require = createRequire(import.meta.url);
const repository = fileURLToPath(new URL("../../", import.meta.url));

export function deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

export function attachment(id) {
  return {
    id,
    name: `${id}.txt`,
    size: 12,
    type: "text/plain",
    kind: "text",
    contentPreview: `contents of ${id}`,
    mediaDataUrl: null,
    mediaMimeType: null,
    mediaItems: [],
  };
}

export function message(owner, id = `${owner}-user`, content = "original prompt", role = "user") {
  return {
    id,
    conversationId: owner,
    content,
    role,
    status: "completed",
    modelId: null,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  };
}

// Exercise production App state and handlers. Only native/API, file decoding and
// child presentation components are doubles; no draft/recovery logic is copied.
export async function createChatAppHarness() {
  const directory = await mkdtemp(join(tmpdir(), "openmind-chat-drafts-"));
  const outfile = join(directory, "App.mjs");
  await build({
    entryPoints: [join(repository, "src/App.tsx")],
    outfile,
    bundle: true,
    platform: "node",
    format: "esm",
    jsx: "automatic",
    plugins: [
      {
        name: "app-boundaries",
        setup(builder) {
          builder.onResolve({ filter: /^react(?:\/jsx-runtime)?$/ }, (args) => ({
          path: pathToFileURL(require.resolve(args.path)).href,
            external: true,
          }));
          builder.onResolve({ filter: /^\.\/api$/ }, () => ({ path: "api", namespace: "fixture" }));
          builder.onResolve({ filter: /^\.\/cloudApi$/ }, () => ({ path: "cloud", namespace: "fixture" }));
          builder.onResolve({ filter: /^\.\/components\// }, (args) => ({
            path: args.path.split("/").at(-1),
            namespace: "fixture",
          }));
          builder.onResolve({ filter: /^@tauri-apps\/api\/event$/ }, () => ({
            path: "events",
            namespace: "fixture",
          }));
          builder.onResolve({ filter: /^socket.io-client$/ }, () => ({
            path: "socket",
            namespace: "fixture",
          }));
          builder.onResolve({ filter: /^lucide-react$/ }, () => ({
            path: "icons",
            namespace: "fixture",
          }));
          builder.onResolve({ filter: /^\.\/lib\/chat$/ }, () => ({
            path: "chat",
            namespace: "fixture",
          }));
          builder.onResolve({ filter: /^\.\/lib\/notify$/ }, () => ({
            path: "notify",
            namespace: "fixture",
          }));
          builder.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => {
            const modules = {
              api: "export const api=globalThis.__chatDraftHarness.api;",
              cloud: "export const cloud={status:async()=>({connected:false,user:null,message:null}),models:async()=>[]};export const hostedChat=async()=>{throw Error('Unexpected Cloud call during local draft regression');};export const submitQueuedScene=async()=>{throw Error('Unexpected queued Studio call during local chat regression');};",
              events:
                "export const listen=async(name,callback)=>{const events=globalThis.__chatDraftHarness.events;events.set(name,callback);return()=>events.delete(name);};",
              socket:
                "export const io=()=>{throw Error('Unexpected socket connection in draft test');};",
              icons:
                "export const Archive=()=>null,Check=()=>null,Pencil=()=>null,Pin=()=>null,PinOff=()=>null,Trash2=()=>null;",
              notify: "export const notifyUser=async()=>{};",
              chat: `export const attachmentMedia=items=>items.flatMap(item=>item.mediaItems),buildMessageContent=content=>content,
              inferChatMode=()=>globalThis.__chatDraftHarness.mode,isUntitledConversation=()=>false,attachmentContextStats=()=>({truncatedCount:0,omittedCount:0}),
              attachmentKind=file=>file.attachmentDraft?.kind??file.kind??"text",
              readAttachment=async (file,options)=>{file.reads=(file.reads??0)+1;file.readOptions=options;if(file.gate)await file.gate.promise;if(file.failure)throw file.failure;return file.attachmentDraft;},settledValue=result=>result.status==='fulfilled'?result.value:null,
              titleFromPrompt=content=>content;`,
            };
            if (modules[path]) return { contents: modules[path], loader: "js" };
            const capture =
              path === "ChatView"
                ? "globalThis.__chatDraftHarness.chat=props;"
                : path === "Sidebar"
                  ? "globalThis.__chatDraftHarness.sidebar=props;"
                  : "";
            return {
              contents: `export function ${path}(props){${capture}return null;}`,
              loader: "js",
            };
          });
        },
      },
    ],
  });
  const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
  const previous = new Map();
  for (const [name, value] of Object.entries({
    window: dom.window,
    document: dom.window.document,
    navigator: dom.window.navigator,
    IS_REACT_ACT_ENVIRONMENT: true,
  })) {
    previous.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
  }
  const { createRoot } = await import("react-dom/client");
  let fixture, root;
  const context = { mode: "chat", events: new Map() };
  context.api = new Proxy(
    {},
    {
      get:
        (_, name) =>
        async (...args) => {
          fixture.calls.push([name, ...args]);
          switch (name) {
            case "root":
              return { root: "/fixture" };
            case "conversations":
              return fixture.conversations;
            case "models":
              return [];
            case "preferences":
              return {
                theme: "light",
                compactSidebar: false,
                localRuntimeAutostart: false,
                notifyModelUpdates: false,
                socketRealtimeEnabled: false,
                autoGenerateTitles: false,
                openArtifactsAfterGeneration: false,
              };
            case "installationStatus":
              return { setupRequired: false };
            case "hardware":
            case "storage":
            case "refreshRuntimeInventory":
            case "runtimeStatus":
            case "performance":
            case "userProfile":
              return null;
            case "messages":
              return globalThis.structuredClone(fixture.messages[args[0]] ?? []);
            case "listArtifacts":
              return [];
            case "sendChatMessage":
              return fixture.send.promise;
            case "regenerateMessage":
              return fixture.regeneration.promise;
            case "createConversation":
              return fixture.create.promise;
            case "forkConversationForEdit":
              return fixture.fork.promise;
            case "createGenerationArtifact":
              return fixture.artifact.promise;
            case "addUserMessage":
              return message(args[0], `${args[0]}-image-user`, args[1]);
            case "createStreamingAssistantMessage":
              return message(args[0], `${args[0]}-image-assistant`, "", "assistant");
            case "appendMessageChunk":
            case "completeMessage":
              return;
            default:
              throw Error(`Unexpected API call in draft test: ${String(name)}`);
          }
        },
    },
  );
  globalThis.__chatDraftHarness = context;
  const { App } = await import(pathToFileURL(outfile).href);
  const act = React.act;
  const harness = {
    get props() {
      return context.chat;
    },
    get fixture() {
      return fixture;
    },
    get text() {
      return dom.window.document.body.textContent;
    },
    async mount() {
      context.mode = "chat";
      fixture = {
        calls: [],
        send: deferred(),
        regeneration: deferred(),
        create: deferred(),
        fork: deferred(),
        artifact: deferred(),
        conversations: ["A", "B"].map((id) => ({
          id,
          title: `${id} chat`,
          activeModelId: null,
          createdAt: "2026-01-01",
          updatedAt: "2026-01-01",
          archivedAt: null,
          pinned: false,
        })),
        messages: { A: [message("A")], B: [message("B")] },
      };
      root = createRoot(dom.window.document.getElementById("root"));
      await act(async () => root.render(React.createElement(App)));
      await harness.open("A");
    },
    async unmount() {
      await act(async () => root.unmount());
    },
    async open(id) {
      await act(async () => context.sidebar.onOpenConversation(id));
    },
    async openTools() {
      await act(async () => context.sidebar.onOpenTools());
    },
    async newChat() {
      await act(async () => context.sidebar.onNewChat());
    },
    async type(text) {
      await act(async () => context.chat.setPrompt(text));
    },
    async addFile(file) {
      await act(async () => context.chat.addFiles([{ attachmentDraft: file }]));
    },
    async removeFile(id) {
      await act(async () => context.chat.removeAttachment(id));
    },
    async edit(id, text) {
      await act(async () => context.chat.editUserMessage(id, text));
    },
    async send(mode = "chat") {
      context.mode = mode;
      let pending;
      // Do not await the request inside act: it intentionally waits on the gate.
      await act(async () => {
        pending = context.chat.sendMessage();
      });
      return { pending, gate: mode === "image" ? fixture.artifact : fixture.send };
    },
    async fail(request, gate = request.gate) {
      await act(async () => {
        gate.reject(Error("fixture request failure"));
        await request.pending;
      });
    },
    async resolve(gate, result) {
      await act(async () => {
        gate.resolve(result);
      });
    },
    async succeed(request, owner = "A") {
      await act(async () => {
        request.gate.resolve(message(owner, `${owner}-reply`, "reply", "assistant"));
        await request.pending;
      });
    },
    async dispose() {
      dom.window.close();
      delete globalThis.__chatDraftHarness;
      for (const [name, descriptor] of previous) {
        if (descriptor) Object.defineProperty(globalThis, name, descriptor);
        else delete globalThis[name];
      }
      await rm(directory, { recursive: true, force: true });
    },
  };
  return harness;
}
