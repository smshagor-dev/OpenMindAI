import assert from "node:assert/strict";
import test from "node:test";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { createRequire } from "node:module";
import React, { act } from "react";
import { JSDOM } from "jsdom";

const require = createRequire(import.meta.url);
const deferred = () => {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};
const delay = (ms) => new Promise((resolve) => globalThis.setTimeout(resolve, ms));

test("first-run storage ignores stale A/B/A responses and exposes offline-drive errors", async () => {
  const directory = await mkdtemp(join(tmpdir(), "storage-preflight-"));
  const dom = new JSDOM('<!doctype html><div id="root"></div>', {
    url: "http://localhost/",
  });
  const descriptors = new Map();
  for (const [name, value] of Object.entries({
    window: dom.window,
    document: dom.window.document,
    navigator: dom.window.navigator,
    IS_REACT_ACT_ENVIRONMENT: true,
  })) {
    descriptors.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value, configurable: true });
  }
  const calls = [];
  globalThis.__storageProbe = (path) => {
    const request = deferred();
    calls.push({ path, ...request });
    return request.promise;
  };

  let root;
  try {
    const outfile = join(directory, "wizard.mjs");
    const fixture = {
      "@tauri-apps/plugin-dialog": "export async function open() { return null; }",
      "@tauri-apps/plugin-process": "export async function relaunch() {}",
      "../api": "export const api = {checkStorageLocation: (path) => globalThis.__storageProbe(path), saveSetupProgress: async () => {}, completeSetup: async () => {}};",
      "../lib/format": "export const formatBytes = (n) => String(n) + ' bytes'; export const formatError = (e) => String(e?.message ?? e);",
      "../lib/notify": "export const notifyUser = async () => {};",
      "./ModelsManager": "export const ModelsManager = () => null;",
      "../cloudApi": "export const cloud = {status: async () => ({connected: false}), login: async () => ({connected: false})};",
      "../lib/setupExecutionMode": "export const setupExecutionMode = () => 'local'; export const beginSetupPreparation = () => {}; export const completeSetupPreparation = () => {};",
    };
    await build({
      entryPoints: ["src/components/SetupWizardDesktop.tsx"],
      outfile,
      bundle: true,
      platform: "node",
      format: "esm",
      jsx: "automatic",
      plugins: [{
        name: "mock-native-setup-boundaries",
        setup(builder) {
          builder.onResolve({ filter: /^react(?:\/jsx-runtime)?$/ }, (args) => ({
            path: pathToFileURL(require.resolve(args.path)).href,
            external: true,
          }));
          builder.onResolve({
            filter: /^(?:@tauri-apps\/plugin-(?:dialog|process)|\.\.\/api|\.\.\/lib\/(?:format|notify|setupExecutionMode)|\.\/ModelsManager|\.\.\/cloudApi)$/,
          }, (args) => ({ path: args.path, namespace: "fixture" }));
          builder.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => ({
            contents: fixture[path],
          }));
        },
      }],
    });

    const { SetupWizard } = await import(pathToFileURL(outfile));
    const { createRoot } = await import("react-dom/client");
    root = createRoot(dom.window.document.getElementById("root"));

    const click = async (label) => {
      const button = [...dom.window.document.querySelectorAll("button")]
        .find((item) => item.textContent.trim() === label);
      assert.ok(button, `Missing button: ${label}`);
      await act(async () => button.click());
      return button;
    };
    const storageInput = () => dom.window.document.querySelector(".setup-path-row input");
    const continueButton = () => [...dom.window.document.querySelectorAll("button")]
      .find((item) => item.textContent.trim() === "Continue");
    const setPath = async (value) => {
      const input = storageInput();
      assert.ok(input, "storage input not shown");
      const setter = Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, "value").set;
      await act(async () => {
        setter.call(input, value);
        input.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
        input.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
      });
    };
    const pendingFor = async (path) => {
      await act(async () => delay(410));
      const matching = calls.filter((call) => call.path === path);
      assert.ok(matching.length, `No validation request for ${path}`);
      return matching.at(-1);
    };
    const respond = async (request, writable = true) => {
      await act(async () => request.resolve({
        path: request.path,
        writable,
        availableBytes: 10_000_000_000,
        recommendedBytes: 6_000_000_000,
      }));
    };

    await act(async () => root.render(React.createElement(SetupWizard, { mode: "full" })));
    await click("Get Started");
    await click("Continue");

    await setPath("D:\\verified");
    const initialA = await pendingFor("D:\\verified");
    await respond(initialA);
    assert.match(dom.window.document.body.textContent, /Location looks writable/);
    assert.equal(continueButton().disabled, false);

    // The old success vanishes synchronously when an offline drive is chosen.
    await setPath("X:\\offline");
    assert.doesNotMatch(dom.window.document.body.textContent, /Location looks writable/);
    assert.equal(continueButton().disabled, true);
    const missing = await pendingFor("X:\\offline");
    await act(async () => missing.reject(new Error("drive disconnected")));
    assert.match(dom.window.document.body.textContent, /drive disconnected/);
    assert.equal(continueButton().disabled, true);

    await setPath("D:\\verified");
    const staleA = await pendingFor("D:\\verified");
    await setPath("X:\\offline");
    const staleB = await pendingFor("X:\\offline");
    await setPath("D:\\verified");
    const currentA = await pendingFor("D:\\verified");
    await respond(staleA);
    assert.doesNotMatch(dom.window.document.body.textContent, /Location looks writable/);
    assert.equal(continueButton().disabled, true);
    await respond(currentA, false);
    assert.match(dom.window.document.body.textContent, /not writable/);
    assert.equal(continueButton().disabled, true);
    await respond(staleB);
    assert.match(dom.window.document.body.textContent, /not writable/);
    assert.equal(continueButton().disabled, true);

    await setPath("D:\\ready");
    const ready = await pendingFor("D:\\ready");
    await respond(ready);
    assert.equal(continueButton().disabled, false);
    await click("Continue");
    assert.match(dom.window.document.body.textContent, /Create Your Local AI Profile/);
  } finally {
    if (root) await act(async () => root.unmount());
    delete globalThis.__storageProbe;
    dom.window.close();
    for (const [name, descriptor] of descriptors) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else delete globalThis[name];
    }
    await rm(directory, { recursive: true, force: true });
  }
});
