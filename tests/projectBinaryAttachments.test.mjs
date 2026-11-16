import test from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { JSDOM } from "jsdom";

const directory = await mkdtemp(join(tmpdir(), "project-binary-"));
const output = join(directory, "binary.mjs");
await build({
  entryPoints: ["src/lib/projectBinaryStorage.ts"],
  outfile: output,
  bundle: true,
  format: "esm",
  platform: "node",
  plugins: [{
    name: "project-binary-api-stub",
    setup(builder) {
      builder.onResolve({ filter: /^\.\.\/api$/ }, () => ({
        path: "api", namespace: "fixture",
      }));
      builder.onLoad({ filter: /.*/, namespace: "fixture" }, () => ({
        contents: "export const api = { addProjectBinaryFile: (...args) => globalThis.__projectBinaryApi.addProjectBinaryFile(...args), readProjectBinaryFile: (...args) => globalThis.__projectBinaryApi.readProjectBinaryFile(...args) };",
      }));
    },
  }],
});
const { attachProjectBinaryFile, saveProjectBinaryCopy } =
  await import(pathToFileURL(output));

const makeFile = (name, bytes, type = "application/octet-stream") => ({
  name, size: bytes.length, type,
  arrayBuffer: async () => Uint8Array.from(bytes).buffer,
});

test("binary upload sends original bytes, not a text preview or a source-device path", async () => {
  const bytes = [0, 255, 137, 80, 78, 71, 0, 28, 19];
  const calls = [];
  globalThis.__projectBinaryApi = {
    addProjectBinaryFile: async (...args) => {
      calls.push(args);
      return { id: "saved-1", status: "attached" };
    },
  };
  try {
    const saved = await attachProjectBinaryFile("project-a", makeFile("diagram.png", bytes, "image/png"));
    assert.equal(saved.status, "attached");
    assert.equal(calls.length, 1);
    assert.deepEqual(calls[0].slice(0, 4), ["project-a", "diagram.png", bytes.length, "image/png"]);
    assert.deepEqual([...globalThis.atob(calls[0][4])].map((c) => c.charCodeAt(0)), bytes);
    assert.equal(Object.keys(makeFile("diagram.png", bytes)).includes("path"), false);
  } finally {
    delete globalThis.__projectBinaryApi;
  }
});

test("1.5 MB attachment chunk encoding succeeds without spreading a huge argument list", async () => {
  const data = Uint8Array.from({ length: 1_500_000 }, (_, index) => index & 255);
  const calls = [];
  globalThis.__projectBinaryApi = {
    addProjectBinaryFile: async (...args) => { calls.push(args); return { status: "attached" }; },
  };
  try {
    await attachProjectBinaryFile("project-big", makeFile("big.bin", data));
    assert.equal(calls.length, 1);
    assert.equal(calls[0][2], 1_500_000);
    assert.equal(globalThis.atob(calls[0][4]).length, 1_500_000);
    await assert.rejects(
      attachProjectBinaryFile("project-big", makeFile("oversized.bin", new Uint8Array(1_500_001))),
      /1\.5 MB/,
    );
    assert.equal(calls.length, 1, "oversized files must be rejected before calling native storage");
  } finally {
    delete globalThis.__projectBinaryApi;
  }
});

test("Save copy returns exact scoped bytes with sanitized filename, and rejects damaged lengths", async () => {
  const dom = new JSDOM("<!doctype html><html><body></body></html>", {
    url: "http://localhost/",
  });
  const originalDocument = globalThis.document;
  const originalURL = globalThis.URL;
  const originalTimer = globalThis.setTimeout;
  const originals = [];
  const downloads = [];
  const requested = [];
  let blobRef;
  const url = {
    createObjectURL(blob) { blobRef = blob; return "blob:verified-download"; },
    revokeObjectURL(value) { originals.push(value); },
  };
  const originalClick = dom.window.HTMLAnchorElement.prototype.click;
  dom.window.HTMLAnchorElement.prototype.click = function() {
    downloads.push({ href: this.href, name: this.download });
  };
  globalThis.document = dom.window.document;
  globalThis.URL = url;
  globalThis.setTimeout = (fn) => { fn(); return 0; };
  globalThis.__projectBinaryApi = {
    readProjectBinaryFile: async (projectId, fileId) => {
      requested.push([projectId, fileId]);
      return {
        name: "../../untrusted\\filename.png",
        mimeType: "image/png",
        dataBase64: globalThis.btoa(String.fromCharCode(0, 255, 137, 80, 78, 71)),
        sizeBytes: 6,
        sha256: "dummy-this-is-verified-in-native-repository",
      };
    },
  };
  try {
    await saveProjectBinaryCopy("owner-project", "file-23");
    assert.deepEqual(requested, [["owner-project", "file-23"]]);
    assert.equal(downloads.length, 1);
    assert.equal(downloads[0].name.includes("/"), false);
    assert.equal(downloads[0].name.includes("\\"), false);
    assert.equal(downloads[0].href, "blob:verified-download");
    assert.deepEqual([...new Uint8Array(await blobRef.arrayBuffer())], [0, 255, 137, 80, 78, 71]);
    assert.deepEqual(originals, ["blob:verified-download"]);
    assert.equal(dom.window.document.querySelectorAll("a").length, 0);

    globalThis.__projectBinaryApi.readProjectBinaryFile = async () => ({
      name: "corrupt.bin", mimeType: null, dataBase64: "AA==", sizeBytes: 30, sha256: "x",
    });
    await assert.rejects(saveProjectBinaryCopy("owner-project", "file-23"), /size validation/);
    assert.equal(downloads.length, 1);
  } finally {
    dom.window.HTMLAnchorElement.prototype.click = originalClick;
    globalThis.document = originalDocument;
    globalThis.URL = originalURL;
    globalThis.setTimeout = originalTimer;
    delete globalThis.__projectBinaryApi;
    dom.window.close();
  }
});

test.after(async () => {
  await rm(directory, { recursive: true, force: true });
});
