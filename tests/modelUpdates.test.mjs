import test, { before, beforeEach, after, afterEach } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import { URL, pathToFileURL, fileURLToPath } from "node:url";
import { build } from "esbuild";
import { JSDOM } from "jsdom";
import React, { act } from "react";

const require = createRequire(import.meta.url);
const repository = fileURLToPath(new URL("../", import.meta.url));
const report = { entries: [{ entry: { id: "lens", name: "OpenMindAI Lens" }, installed: true, updateAvailable: true, compatible: true }] };
let directory, dom, document, root, UpdatesPanel, browserApi, calls, pending, draft;
const previous = new Map();
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

before(async () => {
  directory = await mkdtemp(join(tmpdir(), "openmind-model-updates-"));
  await build({
    entryPoints: [join(repository, "src/components/UpdatesPanel.tsx")],
    outfile: join(directory, "panel.mjs"), bundle: true, platform: "node", format: "esm", jsx: "automatic",
    plugins: [{ name: "native-boundaries", setup(builder) {
      builder.onResolve({ filter: /^react(?:\/jsx-runtime)?$/ }, args => ({ path: pathToFileURL(require.resolve(args.path)).href, external: true }));
      builder.onResolve({ filter: /^(\.\.\/api|\.\/SettingsDialog|\.\.\/lib\/appUpdate|@tauri-apps\/plugin-process)$/ }, args => ({ path: args.path, namespace: "fixture" }));
      builder.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => {
        const modules = {
          "../api": "export const api=new Proxy({}, {get:(_,name)=>(...args)=>globalThis.__modelUpdates.call(name,...args)});",
          "../lib/appUpdate": "export const appUpdateCoordinator={getSnapshot:()=>({state:'idle'}),subscribe:()=>()=>{}};",
          "@tauri-apps/plugin-process": "export const relaunch=()=>{throw Error('Unexpected relaunch');};",
          "./SettingsDialog": `import React from 'react';
            export const ToggleRow=({label,checked,onChange})=>React.createElement('label',null,label,React.createElement('input',{type:'checkbox',checked,onChange:e=>onChange(e.target.checked)}));
            export const SelectRow=()=>null;
            export const SaveBar=({onSave})=>React.createElement('button',{onClick:onSave},'Save');`,
        };
        return { contents: modules[path], loader: "js" };
      });
    } }],
  });
  // Also exercise the real browser fallback preference guard.
  await build({ entryPoints: [join(repository, "src/api_legacy.ts")], outfile: join(directory, "api.mjs"), bundle: true, platform: "node", format: "esm", external: ["@tauri-apps/api/core"] });
  // Resolve the external native module from this repository's dependencies.
  const { symlink } = await import("node:fs/promises");
  await symlink(join(repository, "node_modules"), join(directory, "node_modules"), "dir");
  dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
  document = dom.window.document;
  for (const [name, value] of Object.entries({ window: dom.window, document: dom.window.document, navigator: dom.window.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
    previous.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true });
  }
  ({ UpdatesPanel } = await import(pathToFileURL(join(directory, "panel.mjs"))));
  ({ api: browserApi } = await import(pathToFileURL(join(directory, "api.mjs"))));
});
beforeEach(async () => {
  calls = [];
  pending = deferred();
  draft = { autoCheckAppUpdates: true, autoDownloadAppUpdates: false, notifyModelUpdates: true, autoDownloadModelUpdates: false, updateChannel: "Stable" };
  globalThis.__modelUpdates = { call(name, ...args) {
    calls.push([name, ...args]);
    assert.equal(name, "checkModelUpdates", "checks must never initiate downloads");
    return pending.promise;
  } };
  const { createRoot } = await import("react-dom/client");
  root = createRoot(document.getElementById("root"));
});
afterEach(async () => { await act(async () => root.unmount()); });
after(async () => {
  delete globalThis.__modelUpdates;
  dom.window.close();
  for (const [name, descriptor] of previous) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor);
    else delete globalThis[name];
  }
  await rm(directory, { recursive: true, force: true });
});
async function render() {
  await act(async () => root.render(React.createElement(UpdatesPanel, {
    preferenceDraft: draft, saveState: "idle", onSave: () => {},
    setPreference: (key, value) => { calls.push(["setPreference", key, value]); draft[key] = value; },
  })));
}
function checkButton() { return [...document.querySelectorAll("button")].find(button => /Check for Model Updates|Checking…/.test(button.textContent)); }
async function click(button) { await act(async () => button.click()); }

for (const legacy of [false, true]) {
  test(`automatic model downloading is visibly unavailable with legacy flag ${legacy}`, async () => {
    draft.autoDownloadModelUpdates = legacy;
    await render();
    assert(document.body.textContent.includes("Automatic model downloading is not available"));
    assert(document.body.textContent.includes("Settings → Models"));
    assert(![...document.querySelectorAll("label")].some(label => /large AI model|Automatic.*model/.test(label.textContent)));
    assert.deepEqual(calls, []);
  });
}
test("notification choices stay editable without starting downloads", async () => {
  await render();
  const label = [...document.querySelectorAll("label")].find(label => label.textContent.startsWith("Notify me"));
  await click(label.querySelector("input"));
  assert.deepEqual(calls, [["setPreference", "notifyModelUpdates", false]]);
});
test("duplicate checks are blocked while pending and repeated checks only report updates", async () => {
  await render();
  await click(checkButton());
  assert(checkButton().disabled);
  await click(checkButton());
  assert.deepEqual(calls, [["checkModelUpdates"]]);
  await act(async () => pending.resolve(report));
  assert(document.body.textContent.includes("Update available"));
  pending = deferred();
  await click(checkButton());
  await act(async () => pending.resolve(report));
  assert.deepEqual(calls, [["checkModelUpdates"], ["checkModelUpdates"]]);
});
test("interrupted checks recover and retry without scheduling a download", async () => {
  await render();
  await click(checkButton());
  await act(async () => pending.reject(new Error("interrupted catalog request")));
  assert(!checkButton().disabled);
  pending = deferred();
  await click(checkButton());
  await act(async () => pending.resolve(report));
  assert(document.body.textContent.includes("Update available"));
  assert.deepEqual(calls, [["checkModelUpdates"], ["checkModelUpdates"]]);
});
test("browser fallback rejects obsolete automatic-download requests", async () => {
  const defaults = await browserApi.preferences();
  assert.equal(defaults.autoDownloadModelUpdates, false);
  await assert.rejects(browserApi.savePreferences({ ...defaults, autoDownloadModelUpdates: true }), /Automatic model downloading is not available/);
  assert.equal((await browserApi.savePreferences(defaults)).autoDownloadModelUpdates, false);
});
