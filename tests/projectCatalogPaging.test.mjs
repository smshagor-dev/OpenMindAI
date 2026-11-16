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
const deferred = () => {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

test("project catalog keeps file bodies out of sidebar, paginates and ignores stale details", async () => {
  const folder = await mkdtemp(join(tmpdir(), "project-catalog-"));
  const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: "http://localhost/" });
  const globals = new Map();
  for (const [key, value] of Object.entries({
    window: dom.window,
    document: dom.window.document,
    navigator: dom.window.navigator,
    IS_REACT_ACT_ENVIRONMENT: true,
  })) {
    globals.set(key, Object.getOwnPropertyDescriptor(globalThis, key));
    Object.defineProperty(globalThis, key, { value, configurable: true });
  }

  const summaries = Array.from({ length: 125 }, (_, index) => ({
    id: `p-${index}`,
    name: `Project ${index}`,
    updatedAt: "2026-01-01T00:00:00Z",
    conversationCount: index,
    fileCount: 3,
  }));
  const detailCalls = [];
  const pages = [];
  const detailFor = (id) => ({
    id,
    name: id,
    updatedAt: "2026-01-01T00:00:00Z",
    createdAt: "2026-01-01T00:00:00Z",
    instructions: "CONFIDENTIAL_ONLY_IN_DETAIL",
    conversationIds: [`chat-${id}`],
    files: [{
      id: `file-${id}`,
      projectId: id,
      name: "document.md",
      contentText: "PRIVATE_120KB_BLOB",
      sizeBytes: 100_000,
      status: "ready",
      error: null,
    }],
  });
  globalThis.__projectApi = {
    projects: async (cursor) => {
      pages.push(cursor);
      if (!cursor) return { projects: summaries.slice(0, 50), nextCursor: "after-50" };
      if (cursor === "after-50") return { projects: summaries.slice(50, 100), nextCursor: "after-100" };
      if (cursor === "after-100") return { projects: summaries.slice(100), nextCursor: null };
      throw Error(`unexpected project cursor: ${cursor}`);
    },
    getProject: (projectId) => {
      const request = deferred();
      detailCalls.push({ projectId, ...request });
      return request.promise;
    },
  };

  let root;
  try {
    const outfile = join(folder, "hook.mjs");
    await build({
      entryPoints: ["src/lib/useProjectCatalog.ts"],
      outfile,
      bundle: true,
      format: "esm",
      platform: "node",
      plugins: [{
        name: "mock-project-api",
        setup(builder) {
          builder.onResolve({ filter: /^react(?:\/jsx-runtime)?$/ }, (args) => ({
            path: pathToFileURL(require.resolve(args.path)).href,
            external: true,
          }));
          builder.onResolve({ filter: /^(?:\.\.\/api|\.\/format)$/ }, (args) => ({
            path: args.path, namespace: "fixture",
          }));
          builder.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => ({
            contents: path === "../api"
              ? "export const api = globalThis.__projectApi;"
              : "export const formatError = error => String(error?.message ?? error);",
          }));
        },
      }],
    });
    const { useProjectCatalog } = await import(pathToFileURL(outfile));
    const { createRoot } = await import("react-dom/client");
    let catalog;
    const View = () => { catalog = useProjectCatalog(); return null; };
    root = createRoot(dom.window.document.getElementById("root"));
    await act(async () => root.render(React.createElement(View)));
    assert.equal(catalog.projects.length, 50);
    assert.equal(catalog.nextCursor, "after-50");
    assert.equal(detailCalls.length, 1, "only selected project should retrieve its content");
    assert.equal(detailCalls[0].projectId, "p-0");
    assert.equal(JSON.stringify(catalog.projects).includes("PRIVATE_120KB_BLOB"), false);

    await act(async () => catalog.setActiveId("p-1"));
    assert.equal(detailCalls.length, 2);
    await act(async () => detailCalls[0].resolve(detailFor("p-0")));
    assert.equal(catalog.activeProject, null, "stale detail must not appear in another project");
    await act(async () => detailCalls[1].resolve(detailFor("p-1")));
    assert.equal(catalog.activeProject.id, "p-1");
    assert.equal(catalog.activeProject.files[0].contentText, "PRIVATE_120KB_BLOB");

    await act(async () => catalog.loadMore());
    assert.equal(catalog.projects.length, 100);
    assert.equal(catalog.nextCursor, "after-100");
    assert.equal(detailCalls.length, 2, "loading summaries must not fetch file text");
    await act(async () => catalog.loadMore());
    assert.equal(catalog.projects.length, 125);
    assert.equal(catalog.nextCursor, null);
    assert.deepEqual(pages, [undefined, "after-50", "after-100"]);
    assert.equal(detailCalls.length, 2);

    await act(async () => catalog.setActiveId("p-124"));
    assert.equal(detailCalls.length, 3);
    await act(async () => detailCalls[2].resolve(detailFor("p-124")));
    assert.equal(catalog.activeProject.id, "p-124");
    assert.equal(catalog.activeProject.instructions, "CONFIDENTIAL_ONLY_IN_DETAIL");

    // A new refresh returns only the first 50 metadata rows, but preserves
    // the selected ID and loads its fresh detail without rendering another
    // project's text.
    await act(async () => catalog.refreshProjects());
    assert.equal(catalog.activeId, "p-124");
    assert.equal(catalog.projects.length, 51, "first page plus retained selected project");
    assert(catalog.projects.some((project) => project.id === "p-124"),
      "selected project must remain in the dropdown after refreshing");
    assert.equal(catalog.activeProject, null);
    assert.equal(detailCalls.length, 4);
    await act(async () => detailCalls[3].resolve(detailFor("p-124")));
    assert.equal(catalog.activeProject.id, "p-124");
  } finally {
    if (root) await act(async () => root.unmount());
    delete globalThis.__projectApi;
    dom.window.close();
    for (const [key, descriptor] of globals) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    }
    await rm(folder, { recursive: true, force: true });
  }
});
