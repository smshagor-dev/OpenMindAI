import test from "node:test";
import { Blob } from "node:buffer";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import React, { act } from "react";
import { JSDOM } from "jsdom";
const require = createRequire(import.meta.url);

test("production Composer ignores transcription after discard/unmount and releases microphone tracks", async () => {
  const directory = await mkdtemp(join(tmpdir(), "composer-ime-"));
  const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
  let stopped = 0;
  let recorders = 0;
  let current = true;
  let appended = [];
  const stream = { getTracks: () => [{ stop: () => { stopped++; } }] };
  let permission = Promise.resolve(stream);
  Object.defineProperty(dom.window.navigator, "mediaDevices", { value: { getUserMedia: () => permission } });
  class FakeRecorder {
    state = "inactive";
    mimeType = "audio/webm";
    static isTypeSupported() { return true; }
    constructor() { recorders++; }
    start() { this.state = "recording"; }
    stop() {
      if (this.state !== "recording") return;
      this.state = "inactive";
      this.ondataavailable?.({ data: new Blob(["fixture audio"]) });
      this.onstop?.();
    }
  }
  const previous = new Map();
  for (const [name, value] of Object.entries({ window: dom.window, document: dom.window.document, navigator: dom.window.navigator, IS_REACT_ACT_ENVIRONMENT: true, MediaRecorder: FakeRecorder })) {
    previous.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value, configurable: true });
  }
  let root;
  try {
    const outfile = join(directory, "composer.mjs");
    await build({ entryPoints: ["src/components/Composer.tsx"], outfile, bundle: true, platform: "node", format: "esm", jsx: "automatic", plugins: [{ name: "native-boundaries", setup(builder) {
      builder.onResolve({ filter: /^react(?:\/jsx-runtime)?$/ }, args => ({ path: pathToFileURL(require.resolve(args.path)).href, external: true }));
      builder.onResolve({ filter: /^lucide-react$/ }, () => ({ path: "icons", namespace: "fixture" }));
      builder.onResolve({ filter: /^(\.\.\/api|\.\.\/lib\/media|\.\.\/lib\/chat|\.\/ModelSelector)$/ }, args => ({ path: args.path, namespace: "fixture" }));
      builder.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => ({ contents: {
        "icons": "export const FileText=()=>null,FileType=()=>null,Hash=()=>null,Image=()=>null,Loader2=()=>null,Mic=()=>null,MicOff=()=>null,Music2=()=>null,Paperclip=()=>null,Plus=()=>null,Send=()=>null,Square=()=>null,Video=()=>null,Volume2=()=>null,X=()=>null;",
        "../api": "export const api={checkModelUpdates:async()=>({entries:[]})};",
        "../lib/media": "export const transcribeAudioBlob=(...args)=>globalThis.__voiceTranscribe(...args);",
        "../lib/chat": "export const inferChatMode=()=> 'chat';",
        "./ModelSelector": "export const ModelSelector=()=>null;",
      }[path] }));
    } }] });
    const { Composer } = await import(pathToFileURL(outfile));
    const { createRoot } = await import("react-dom/client");
    root = createRoot(dom.window.document.getElementById("root"));
    const props = { prompt: "note", setPrompt: () => { throw Error("stale setPrompt must not be used"); }, createVoiceDraftTarget: () => ({ isCurrent: () => current, append: text => { if (!current) return false; appended.push(text); return true; } }), imageOptions: {}, setImageOptions: () => {}, attachments: [], enterToSend: true, streaming: false, submitting: false, addFiles: () => {}, removeAttachment: () => {}, sendMessage: () => {}, stopGeneration: () => {}, composerRef: React.createRef(), models: [], activeModelId: null, runtime: null, modelSwitching: false, modelSwitchError: null, onSelectModel: () => {} };
    const render = async () => { await act(async () => root.render(React.createElement(Composer, props))); };
    const click = async title => { await act(async () => dom.window.document.querySelector(`button[title="${title}"]`).click()); };
    const defer = () => { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; };
    let recognition = defer();
    globalThis.__voiceTranscribe = () => recognition.promise;
    await render(); await click("Voice input"); await click("Stop recording");
    await act(async () => recognition.resolve({ text: "successful speech" }));
    assert.deepEqual(appended, ["successful speech"]);
    appended = [];
    recognition = defer();
    await click("Voice input"); await click("Stop recording");
    current = false;
    await act(async () => recognition.resolve({ text: "discarded speech" }));
    assert.deepEqual(appended, []);
    current = true;
    recognition = defer();
    await click("Voice input"); await click("Stop recording");
    await act(async () => root.unmount()); root = null;
    await act(async () => recognition.resolve({ text: "unmounted speech" }));
    assert.deepEqual(appended, []);
    // A permission request resolving after unmount must stop its stream and
    // must never construct a recorder or start transcription.
    const pendingPermission = defer(); permission = pendingPermission.promise;
    root = createRoot(dom.window.document.getElementById("root"));
    await render(); await click("Voice input");
    const before = recorders;
    await act(async () => root.unmount()); root = null;
    await act(async () => pendingPermission.resolve(stream));
    assert.equal(recorders, before);
    assert(stopped >= 4);
  } finally {
    if (root) await act(async () => root.unmount());
    delete globalThis.__voiceTranscribe;
    dom.window.close();
    for (const [name, descriptor] of previous) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor); else delete globalThis[name];
    }
    await rm(directory, { recursive: true, force: true });
  }
});
