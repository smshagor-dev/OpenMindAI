import test from "node:test";
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

test("production Composer preserves IME confirmation and normal send behavior", async () => {
  const directory = await mkdtemp(join(tmpdir(), "composer-ime-"));
  const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
  const previous = new Map();
  for (const [name, value] of Object.entries({ window: dom.window, document: dom.window.document, navigator: dom.window.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
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
        "../lib/media": "export const transcribeAudioBlob=()=>{throw Error('unexpected recording');};",
        "../lib/chat": "export const inferChatMode=()=> 'chat';",
        "./ModelSelector": "export const ModelSelector=()=>null;",
      }[path] }));
    } }] });
    const { Composer } = await import(pathToFileURL(outfile));
    const { createRoot } = await import("react-dom/client");
    root = createRoot(dom.window.document.getElementById("root"));
    let sent = 0;
    const props = { prompt: "বাংলা", setPrompt: () => {}, imageOptions: {}, setImageOptions: () => {}, attachments: [], enterToSend: true, streaming: false, submitting: false, addFiles: () => {}, removeAttachment: () => {}, sendMessage: () => { sent++; }, stopGeneration: () => {}, composerRef: React.createRef(), models: [], activeModelId: null, runtime: null, modelSwitching: false, modelSwitchError: null, onSelectModel: () => {} };
    const render = async (changes = {}) => { Object.assign(props, changes); await act(async () => root.render(React.createElement(Composer, props))); };
    await render();
    const textarea = () => dom.window.document.querySelector("textarea");
    const key = async (options = {}) => {
      const event = new dom.window.KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true, ...options });
      await act(async () => textarea().dispatchEvent(event));
      return event;
    };
    for (const prompt of ["বাংলা", "日本語", "中文"]) {
      await render({ prompt });
      assert.equal((await key({ isComposing: true })).defaultPrevented, false);
      assert.equal(sent, 0);
      assert.equal(textarea().value, prompt);
    }
    // Some WebViews report false isComposing during an active composition.
    await act(async () => textarea().dispatchEvent(new dom.window.CompositionEvent("compositionstart", { bubbles: true })));
    assert.equal((await key()).defaultPrevented, false);
    assert.equal(sent, 0);
    await act(async () => textarea().dispatchEvent(new dom.window.CompositionEvent("compositionend", { bubbles: true })));
    // WebKit can signal the IME-confirmation Enter only through code 229.
    assert.equal((await key({ keyCode: 229 })).defaultPrevented, false);
    assert.equal(sent, 0);
    assert.equal((await key()).defaultPrevented, true);
    assert.equal(sent, 1);
    assert.equal((await key({ shiftKey: true })).defaultPrevented, false);
    await render({ enterToSend: false }); await key(); assert.equal(sent, 1);
    await render({ enterToSend: true, streaming: true }); await key(); assert.equal(sent, 1);
    await render({ streaming: false, submitting: true }); await key(); assert.equal(sent, 1);
    await render({ submitting: false, prompt: "" }); await key(); assert.equal(sent, 1);
  } finally {
    if (root) await act(async () => root.unmount());
    dom.window.close();
    for (const [name, descriptor] of previous) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor); else delete globalThis[name];
    }
    await rm(directory, { recursive: true, force: true });
  }
});
