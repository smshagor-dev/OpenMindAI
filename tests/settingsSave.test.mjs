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

test("production settings/profile saves retain drafts, show failures and allow retry", async () => {
  const directory = await mkdtemp(join(tmpdir(), "settings-save-"));
  const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
  const previous = new Map();
  for (const [name, value] of Object.entries({ window: dom.window, document: dom.window.document, navigator: dom.window.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
    previous.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value, configurable: true });
  }
  let root;
  try {
    const outfile = join(directory, "settings.mjs");
    await build({ entryPoints: ["src/components/SettingsDialog.tsx"], outfile, bundle: true, platform: "node", format: "esm", jsx: "automatic", plugins: [{ name: "boundaries", setup(builder) {
      builder.onResolve({ filter: /^react(?:\/jsx-runtime)?$/ }, args => ({ path: pathToFileURL(require.resolve(args.path)).href, external: true }));
      builder.onResolve({ filter: /^lucide-react$/ }, () => ({ path: "icons", namespace: "fixture" }));
      builder.onResolve({ filter: /^@tauri-apps\/api\/core$/ }, () => ({ path: "native", namespace: "fixture" }));
      builder.onResolve({ filter: /^\.\/(ModelsManager|AppsSettings|MaintenanceCenter|UpdatesPanel|AgentSettings|DiffusionRuntimePanel)$/ }, args => ({ path: args.path.slice(2), namespace: "fixture" }));
      builder.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => ({ contents: path === "icons"
        ? "export const Brain=()=>null,Cpu=()=>null,Database=()=>null,Download=()=>null,FolderOpen=()=>null,Globe=()=>null,HardDrive=()=>null,Info=()=>null,Keyboard=()=>null,LoaderCircle=()=>null,MessageSquarePlus=()=>null,Palette=()=>null,Play=()=>null,Plug=()=>null,RefreshCw=()=>null,Server=()=>null,Settings=()=>null,Shield=()=>null,SlidersHorizontal=()=>null,StopCircle=()=>null,Trash2=()=>null,User=()=>null,Wrench=()=>null,Bot=()=>null;"
        : path === "native" ? "export const invoke=async()=>{throw Error('unexpected native action');};" : `export const ${path}=()=>null;` }));
    } }] });
    const { SettingsDialog } = await import(pathToFileURL(outfile));
    const { createRoot } = await import("react-dom/client");
    const defer = () => { let resolve, reject; const promise = new Promise((yes,no) => { resolve=yes; reject=no; }); return { promise, resolve, reject }; };
    for (const section of ["general", "personalization", "network", "privacy"]) {
      root = createRoot(dom.window.document.getElementById("root"));
      let pending = defer(); const requests = [];
      const save = value => { requests.push(value); return pending.promise; };
      const props = { initialSection: section, preferences: { language: "English", compactSidebar: false, confirmBeforeDelete: true, networkProxyMode: "direct", networkProxyUrl: "" }, userProfile: { fullName: "Fixture User", preferredName: "", email: "", avatarDataUrl: "", about: "", occupation: "", responseStyle: "", customInstructions: "" }, root: null, hardware: null, storage: null, models: [], runtime: null, runtimeStatus: null, performance: null, updatePreferences: save, updateUserProfile: save, refresh: () => {}, startRuntime: () => {}, stopRuntime: () => {}, runtimeAction: null, runtimeInstallStatus: null, runtimeInstalling: false, runtimeInstallError: null, installRuntime: () => {}, cancelRuntimeInstall: () => {} };
      await act(async () => root.render(React.createElement(SettingsDialog, props)));
      if (section === "network") {
        props.preferences.networkProxyMode = "manual";
        props.preferences.networkProxyUrl = "http://proxy.example:3128/";
        props.preferences.networkProxyCredentialId = "synthetic-reference";
        await act(async () => root.unmount());
        root = createRoot(dom.window.document.getElementById("root"));
        await act(async () => root.render(React.createElement(SettingsDialog, props)));
        const proxyInput = [...dom.window.document.querySelectorAll("label")].find(label => label.textContent.includes("Proxy address")).querySelector("input");
        assert.equal(proxyInput.type, "password", "entered proxy userinfo must be masked");
        assert.equal(proxyInput.value, "http://proxy.example:3128/");
        const unlink = [...dom.window.document.querySelectorAll("button")].find(button => button.textContent.includes("Stop using saved proxy"));
        assert.ok(unlink);
        await act(async () => unlink.click());
      }
      if (section === "privacy") {
        assert.match(dom.window.document.body.textContent, /Not available in this release/);
        assert.match(dom.window.document.body.textContent, /previous switch had no runtime/);
        assert.equal(
          [...dom.window.document.querySelectorAll("label")]
            .some((label) => /telemetry/i.test(label.textContent) && label.querySelector('input[type="checkbox"]')),
          false,
          "privacy must not present an interactive telemetry switch",
        );
      }
      const saveButton = () => dom.window.document.querySelector(".save-bar button");
      const clickSave = async () => { await act(async () => saveButton().click()); };
      await clickSave(); assert(saveButton().disabled);
      if (section === "general") {
        const select = dom.window.document.querySelector("select");
        await act(async () => { select.value = "Bangla"; select.dispatchEvent(new dom.window.Event("change", { bubbles: true })); });
        assert(saveButton().disabled, "editing must not permit a duplicate pending save");
      }
      await clickSave(); assert.equal(requests.length, 1);
      if (section === "network") assert.equal(requests[0].networkProxyCredentialId, null);
      await act(async () => pending.reject(new Error("fixture disk unavailable")));
      assert(!saveButton().disabled);
      assert.match(dom.window.document.querySelector('[role="alert"]').textContent, /fixture disk unavailable/);
      assert(!dom.window.document.querySelector(".save-bar span").textContent.includes("Saved"));
      if (section === "personalization") assert.equal(dom.window.document.querySelector('.settings-content input[type="text"]').value, "Fixture User");
      else if (section === "general") assert.equal(dom.window.document.querySelector("select").value, "Bangla");
      pending = defer(); await clickSave();
      assert.equal(dom.window.document.querySelector('[role="alert"]'), null);
      await act(async () => pending.resolve());
      assert(!saveButton().disabled);
      assert.equal(dom.window.document.querySelector(".save-bar span").textContent, "Saved");
      assert.equal(requests.length, 2);
      if (section === "general") assert.equal(requests[1].language, "Bangla");
      await act(async () => root.unmount()); root = null;
    }
  } finally {
    if (root) await act(async () => root.unmount()); dom.window.close();
    for (const [name, descriptor] of previous) { if (descriptor) Object.defineProperty(globalThis, name, descriptor); else delete globalThis[name]; }
    await rm(directory, { recursive: true, force: true });
  }
});
