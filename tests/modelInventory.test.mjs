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

test("production model selector excludes unavailable records and restores them after rediscovery", async () => {
  const directory = await mkdtemp(join(tmpdir(), "model-inventory-"));
  const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
  const previous = new Map();
  for (const [name, value] of Object.entries({ window: dom.window, document: dom.window.document, navigator: dom.window.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
    previous.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value, configurable: true });
  }
  let root;
  try {
    const outfile = join(directory, "selector.mjs");
    await build({ entryPoints: ["src/components/ModelSelector.tsx"], outfile, bundle: true, platform: "node", format: "esm", jsx: "automatic", plugins: [{ name: "presentation-boundaries", setup(builder) {
      builder.onResolve({ filter: /^react(?:\/jsx-runtime)?$/ }, args => ({ path: pathToFileURL(require.resolve(args.path)).href, external: true }));
      builder.onResolve({ filter: /^lucide-react$/ }, () => ({ path: "icons", namespace: "fixture" }));
      builder.onLoad({ filter: /.*/, namespace: "fixture" }, () => ({ contents: "export const ChevronDown=()=>null,Check=()=>null,Loader2=()=>null;" }));
    } }] });
    const { ModelSelector } = await import(pathToFileURL(outfile));
    const { createRoot } = await import("react-dom/client");
    root = createRoot(dom.window.document.getElementById("root"));
    const missing = { id: "missing", name: "OpenMindAI Missing", path: "missing.gguf", enabled: false, state: "notInstalled" };
    const valid = { id: "valid", name: "OpenMindAI Valid", path: "valid.gguf", enabled: true, state: "ready" };
    const selected = [];
    const props = { models: [missing, valid], activeModelId: "missing", runtime: null, switching: false, switchError: null, onSelect: id => selected.push(id) };
    const render = async () => act(async () => root.render(React.createElement(ModelSelector, props)));
    const click = async element => act(async () => element.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true })));
    await render();
    const trigger = () => dom.window.document.querySelector(".model-selector-trigger");
    assert.match(trigger().textContent, /Missing.*unavailable/);
    await click(trigger());
    let options = [...dom.window.document.querySelectorAll('[role="option"]')];
    assert.equal(options.length, 1);
    assert.match(options[0].textContent, /Valid/);
    await click(options[0]);
    assert.deepEqual(selected, ["valid"]);
    props.models = [missing]; await render(); assert.equal(trigger().disabled, true);
    props.models = [{ ...missing, enabled: true, state: "ready" }, valid]; await render();
    assert.equal(trigger().disabled, false); assert.doesNotMatch(trigger().textContent, /unavailable/);
    await click(trigger()); options = [...dom.window.document.querySelectorAll('[role="option"]')];
    const restored = options.find(option => option.textContent.includes("Missing"));
    assert.ok(restored); await click(restored); assert.deepEqual(selected, ["valid", "missing"]);
  } finally {
    if (root) await act(async () => root.unmount());
    dom.window.close();
    for (const [name, descriptor] of previous) { if (descriptor) Object.defineProperty(globalThis, name, descriptor); else delete globalThis[name]; }
    await rm(directory, { recursive: true, force: true });
  }
});
