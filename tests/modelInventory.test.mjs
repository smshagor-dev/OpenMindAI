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

test("production selector preserves availability and distinct model identities", async (t) => {
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

    await t.test("unknown custom GGUFs remain independently selectable", async () => {
      props.models = [
        { ...valid, id: "custom-a", name: "Local alpha", path: "/models/alpha.gguf", quantization: "Q4_K_M", sizeBytes: 1024 ** 3 },
        { ...valid, id: "custom-b", name: "Local beta", path: "C:\\models\\beta.gguf", quantization: "Q4_K_M", sizeBytes: 1024 ** 3 },
      ];
      props.activeModelId = "custom-b"; await render();
      assert.match(trigger().textContent, /beta\.gguf/);
      await click(trigger());
      options = [...dom.window.document.querySelectorAll('[role="option"]')];
      assert.equal(options.length, 2);
      assert.equal(new Set(options.map(option => option.textContent)).size, 2);
      const alpha = options.find(option => option.textContent.includes("alpha.gguf"));
      assert.ok(alpha); assert.equal(alpha.getAttribute("aria-selected"), "false");
      assert.equal(options.find(option => option.textContent.includes("beta.gguf")).getAttribute("aria-selected"), "true");
      await click(alpha); assert.equal(selected.at(-1), "custom-a");
    });
    await t.test("family quantizations expose their filename and size", async () => {
      props.models = [
        { ...valid, id: "q4", name: "Qwen", sourceRepository: "Qwen/Qwen3-4B-GGUF", path: "/models/core-q4.gguf", quantization: "Q4_K_M", sizeBytes: 2 * 1024 ** 3 },
        { ...valid, id: "q8", name: "Qwen", sourceRepository: "Qwen/Qwen3-4B-GGUF", path: "/models/core-q8.gguf", quantization: "Q8_0", sizeBytes: 4 * 1024 ** 3 },
      ];
      props.activeModelId = "q8"; await render();
      assert.match(trigger().textContent, /Q8_0.*4\.0 GB.*core-q8/);
      for (const [id, quant] of [["q4", "Q4_K_M"], ["q8", "Q8_0"]]) {
        await click(trigger()); options = [...dom.window.document.querySelectorAll('[role="option"]')];
        assert.equal(options.length, 2);
        const option = options.find(item => item.textContent.includes(quant));
        assert.ok(option); assert.match(option.textContent, /OpenMindAI Core/);
        await click(option); assert.equal(selected.at(-1), id);
        props.activeModelId = id; await render();
      }
    });
    await t.test("identical filenames and quantizations remain distinct by stable ID", async () => {
      props.models = [
        { ...valid, id: "same-file-a", name: "Local", path: "/first/model.gguf", quantization: "Q4_K_M", sizeBytes: 1024 },
        { ...valid, id: "same-file-b", name: "Local", path: "/second/model.gguf", quantization: "Q4_K_M", sizeBytes: 1024 },
      ];
      props.activeModelId = "same-file-b"; await render();
      assert.match(trigger().textContent, /same-file-b/);
      await click(trigger()); options = [...dom.window.document.querySelectorAll('[role="option"]')];
      assert.equal(options.length, 2);
      assert.equal(new Set(options.map(option => option.textContent)).size, 2);
      await click(options.find(option => option.textContent.includes("same-file-a")));
      assert.equal(selected.at(-1), "same-file-a");
      props.activeModelId = "same-file-a";
      props.models.reverse(); await render(); await click(trigger());
      options = [...dom.window.document.querySelectorAll('[role="option"]')];
      assert.equal(options.find(option => option.getAttribute("aria-selected") === "true").textContent.includes("same-file-a"), true);
    });
  } finally {
    if (root) await act(async () => root.unmount());
    dom.window.close();
    for (const [name, descriptor] of previous) { if (descriptor) Object.defineProperty(globalThis, name, descriptor); else delete globalThis[name]; }
    await rm(directory, { recursive: true, force: true });
  }
});
