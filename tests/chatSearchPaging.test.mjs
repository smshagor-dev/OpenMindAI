import test from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { createRequire } from "node:module";
import React, { act } from "react";
import { JSDOM } from "jsdom";

const require = createRequire(import.meta.url);
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const sleep = (ms) => new Promise((resolve) => globalThis.setTimeout(resolve, ms));

test("chat search never fetches entire histories and ignores stale or failed searches", async () => {
  const dir = await mkdtemp(join(tmpdir(), "chat-search-"));
  const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
  const previous = new Map();
  for (const [name, value] of Object.entries({
    window: dom.window, document: dom.window.document, navigator: dom.window.navigator,
    IS_REACT_ACT_ENVIRONMENT: true,
  })) {
    previous.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value, configurable: true });
  }
  const requests = [];
  let historyCalls = 0;
  globalThis.__chatSearch = (query, cursor, limit) => {
    const request = deferred();
    requests.push({ query, cursor, limit, ...request });
    return request.promise;
  };
  globalThis.__historyRead = () => {
    historyCalls++;
    throw Error("search must not retrieve full chat history");
  };

  let root = null;
  try {
    const output = join(dir, "search.mjs");
    await build({
      entryPoints: ["src/components/ChatSearch.tsx"],
      outfile: output,
      bundle: true,
      platform: "node",
      format: "esm",
      jsx: "automatic",
      plugins: [{
        name: "mock-chat-search-boundaries",
        setup(builder) {
          builder.onResolve({ filter: /^react(?:\/jsx-runtime)?$/ }, (args) => ({
            path: pathToFileURL(require.resolve(args.path)).href,
            external: true,
          }));
          builder.onResolve({ filter: /^(?:lucide-react|\.\.\/api)$/ }, (args) => ({
            path: args.path, namespace: "fixture",
          }));
          builder.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => ({
            contents: path === "../api"
              ? "export const api = {searchConversations: (...args) => globalThis.__chatSearch(...args), messages: () => globalThis.__historyRead()};"
              : "export const MessageSquare = () => null;",
          }));
        },
      }],
    });

    const { ChatSearch } = await import(pathToFileURL(output));
    const { createRoot } = await import("react-dom/client");
    root = createRoot(dom.window.document.getElementById("root"));
    const existing = Array.from({ length: 500 }, (_, i) => ({
      id: String(i), title: `Chat ${i}`, updatedAt: "2026-01-01",
    }));
    const props = { open: true, conversations: existing, onSelect: () => {}, onClose: () => {} };
    await act(async () => root.render(React.createElement(ChatSearch, props)));
    await act(async () => sleep(20));
    assert.equal(historyCalls, 0);
    assert(requests.length >= 1 && requests.length <= 3, "opening search makes only one bounded request, not 500 history reads");
    assert(requests.every((request) => request.limit === 40));
    for (const request of requests) {
      await act(async () => request.resolve({ conversations: [], nextCursor: null }));
    }

    const change = async (value) => {
      const input = dom.window.document.querySelector(".search-input");
      assert(input);
      const setter = Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, "value").set;
      await act(async () => {
        setter.call(input, value);
        input.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
        input.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
      });
    };
    const last = (query) => {
      const match = requests.filter((item) => item.query === query).at(-1);
      assert(match, `Missing query ${query}`);
      return match;
    };
    const button = (label) => {
      const b = [...dom.window.document.querySelectorAll("button")]
        .find((item) => item.textContent.trim() === label);
      assert(b, `Missing ${label} button`);
      return b;
    };

    await change("older");
    await act(async () => sleep(280));
    const stale = last("older");
    await change("updated");
    await act(async () => sleep(280));
    const current = last("updated");
    await act(async () => stale.resolve({ conversations: [{ id: "stale", title: "Stale reply" }], nextCursor: null }));
    assert.doesNotMatch(dom.window.document.body.textContent, /Stale reply/);
    await act(async () => current.resolve({
      conversations: [{ id: "new", title: "Fresh indexed result" }],
      nextCursor: "2026-01-01T00:00:00Z|00000000-0000-4000-8000-000000000001",
    }));
    assert.match(dom.window.document.body.textContent, /Fresh indexed result/);
    assert.equal(historyCalls, 0);
    assert.equal(button("Next").disabled, false);

    await act(async () => button("Next").click());
    await act(async () => sleep(10));
    const nextPage = requests.filter((item) => item.query === "updated").at(-1);
    assert.equal(nextPage.cursor, "2026-01-01T00:00:00Z|00000000-0000-4000-8000-000000000001");
    await act(async () => nextPage.reject(new Error("temporary database lock")));
    assert.match(dom.window.document.body.textContent, /temporary database lock/);
    assert.equal(button("Next").disabled, true);

    await act(async () => button("Previous").click());
    await act(async () => sleep(10));
    assert.equal(requests.filter((item) => item.query === "updated").at(-1).cursor, undefined);
    assert.equal(historyCalls, 0);
  } finally {
    if (root) await act(async () => root.unmount());
    delete globalThis.__chatSearch;
    delete globalThis.__historyRead;
    dom.window.close();
    for (const [key, desc] of previous) {
      if (desc) Object.defineProperty(globalThis, key, desc);
      else delete globalThis[key];
    }
    await rm(dir, { recursive: true, force: true });
  }
});
