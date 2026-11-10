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

test("model validation/planning, pause, cancel and deletion recover from failure and permit retry", async () => {
  const directory = await mkdtemp(join(tmpdir(), "model-actions-"));
  const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
  const previous = new Map();
  for (const [name, value] of Object.entries({ window: dom.window, document: dom.window.document, navigator: dom.window.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
    previous.set(name, Object.getOwnPropertyDescriptor(globalThis, name)); Object.defineProperty(globalThis, name, { value, configurable: true });
  }
  let root, handler;
  globalThis.__modelActionApi = new Proxy({}, { get: (_, name) => (...args) => handler(name, ...args) });
  try {
    const outfile = join(directory, "models.mjs");
    await build({ entryPoints: ["src/components/ModelsManager.tsx"], outfile, bundle: true, platform: "node", format: "esm", jsx: "automatic", plugins: [{ name: "boundaries", setup(builder) {
      builder.onResolve({ filter: /^react(?:\/jsx-runtime)?$/ }, args => ({ path: pathToFileURL(require.resolve(args.path)).href, external: true }));
      builder.onResolve({ filter: /^lucide-react$/ }, () => ({ path: "icons", namespace: "fixture" }));
      builder.onResolve({ filter: /^\.\.\/api$/ }, () => ({ path: "api", namespace: "fixture" }));
      builder.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => ({ contents: path === "icons" ? "export const CheckCircle2=()=>null,Download=()=>null,PauseCircle=()=>null,RefreshCw=()=>null,StopCircle=()=>null,Trash2=()=>null;" : "export const api=globalThis.__modelActionApi;" }));
    } }] });
    const { ModelsManager } = await import(pathToFileURL(outfile));
    const { createRoot } = await import("react-dom/client");
    const defer = () => { let resolve, reject; const promise = new Promise((yes,no) => { resolve=yes; reject=no; }); return { promise, resolve, reject }; };
    const plan = { config: { contextSize: 100, gpuLayers: 0, threads: 1, flashAttention: false } };
    for (const action of ["validateModel", "planModelLaunch", "pauseModelDownload", "cancelModelDownload", "deleteCatalogModel"]) {
      let pending = defer(); let calls=0;
      const status = { modelId: "fixture", state: "downloading", percentage: 20, downloadedBytes: 20, totalBytes: 100 };
      const catalog = { entries: [{ entry: { id: "fixture", name: "Fixture model", kind: "chat", family: "qwen3", repo: "fixture/repo", quantization: "Q4", sizeBytes: 100, minRamBytes: 0, capabilities: [], license: "Apache-2.0", description: "fixture" }, compatible: true, installed: action === "deleteCatalogModel", downloadSupported: true, updateAvailable: false }] };
      handler = async name => {
        if (name === action) { calls++; return pending.promise; }
        if (name === "checkModelUpdates") return catalog;
        if (name === "modelDownloadStatus") return status;
        if (name === "validateModel") return {};
        if (name === "planModelLaunch") return plan;
        throw Error(`Unexpected API ${name}`);
      };
      root = createRoot(dom.window.document.getElementById("root"));
      await act(async () => root.render(React.createElement(ModelsManager, { hardware: null, models: [{ id: "fixture" }], runtime: null, root: null, refresh: async () => {} })));
      const validateButton = () => dom.window.document.querySelector('[title="Validate and plan first chat model"]');
      const button = () => action === "deleteCatalogModel"
        ? dom.window.document.querySelector('[role="alertdialog"] .danger-button')
        : action === "pauseModelDownload" ? dom.window.document.querySelector('[title="Pause download"]')
        : action === "cancelModelDownload" ? dom.window.document.querySelector('[title="Cancel download"]') : validateButton();
      if (action === "deleteCatalogModel") await act(async () => dom.window.document.querySelector('[title="Delete Fixture model"]').click());
      await act(async () => button().click());
      assert(validateButton().disabled, action);
      await act(async () => button().click()); assert.equal(calls,1,action);
      await act(async () => pending.reject({ code: "Fixture", message: "fixture action failure" }));
      assert(!validateButton().disabled, action);
      assert.match(dom.window.document.querySelector('[role="alert"]').textContent,/fixture action failure/);
      assert(!button().disabled,action);
      pending=defer();await act(async()=>button().click());assert.equal(calls,2,action);
      await act(async()=>pending.resolve(action === "planModelLaunch" ? plan : { ...status, state: "pausedInterrupted" }));
      assert(!validateButton().disabled,action);
      assert.equal(dom.window.document.querySelector('[role="alert"]'),null);
      if(action === "deleteCatalogModel") assert.equal(dom.window.document.querySelector('[role="alertdialog"]'),null);
      await act(async()=>root.unmount());root=null;
    }
  } finally {
    if(root) await act(async()=>root.unmount());dom.window.close();delete globalThis.__modelActionApi;
    for(const [name,descriptor] of previous){if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name];}
    await rm(directory,{recursive:true,force:true});
  }
});
