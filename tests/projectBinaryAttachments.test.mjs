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

test("binary upload rejects oversized metadata before reading bytes or invoking native storage", async () => {
  let reads = 0;
  let writes = 0;
  globalThis.__projectBinaryApi = {
    addProjectBinaryFile: async () => { writes++; return { status: "attached" }; },
  };
  const fixture = (name, type) => ({
    name, type, size: 1,
    arrayBuffer: async () => { reads++; return Uint8Array.of(42).buffer; },
  });
  try {
    await assert.rejects(
      attachProjectBinaryFile("p", fixture("🚀".repeat(64), "image/png")),
      /filename.*255 UTF-8 bytes/,
    );
    await assert.rejects(
      attachProjectBinaryFile("p", fixture("valid.bin", "x".repeat(256))),
      /MIME type/,
    );
    await assert.rejects(
      attachProjectBinaryFile("p", fixture("valid.bin", "image/png\nunsafe")),
      /MIME type/,
    );
    await assert.rejects(
      attachProjectBinaryFile("p", fixture("  ", "image/png")),
      /filename/,
    );
    assert.equal(reads, 0, "invalid metadata must be rejected before file IO");
    assert.equal(writes, 0, "invalid metadata must not reach native storage");

    const accepted = await attachProjectBinaryFile(
      "p", fixture("বাংলা-নথি.bin", "application/octet-stream"),
    );
    assert.equal(accepted.status, "attached");
    assert.equal(reads, 1);
    assert.equal(writes, 1);
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
        name: "../../বাংলা\\invoice\u202Egnp.exe\u2066\u2029.png",
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
    assert.equal(downloads[0].name, ".._.._বাংলা_invoice_gnp.exe__.png");
    assert.doesNotMatch(downloads[0].name, /[\u061c\u200e\u200f\u202a-\u202e\u2028\u2029\u2066-\u2069]/u);
    assert.equal(downloads[0].href, "blob:verified-download");
    assert.deepEqual([...new Uint8Array(await blobRef.arrayBuffer())], [0, 255, 137, 80, 78, 71]);
    assert.deepEqual(originals, ["blob:verified-download"]);
    assert.equal(dom.window.document.querySelectorAll("a").length, 0);

    // An oversized legacy name must not become a multi-megabyte DOM attribute.
    // The code-point boundary must not split a supplementary-plane character.
    globalThis.__projectBinaryApi.readProjectBinaryFile = async () => ({
      name: "x".repeat(254) + "🚀" + "y".repeat(200_000),
      mimeType: "application/octet-stream",
      dataBase64: "AA==", sizeBytes: 1, sha256: "x",
    });
    await saveProjectBinaryCopy("owner-project", "file-23");
    assert.equal(downloads.length, 2);
    assert.equal([...downloads[1].name].length, 255);
    assert.equal(downloads[1].name, "x".repeat(254) + "🚀");
    assert.deepEqual([...new Uint8Array(await blobRef.arrayBuffer())], [0]);
    assert.equal(dom.window.document.querySelectorAll("a").length, 0);

    globalThis.__projectBinaryApi.readProjectBinaryFile = async () => ({
      name: "", mimeType: null, dataBase64: "AA==", sizeBytes: 1, sha256: "x",
    });
    await saveProjectBinaryCopy("owner-project", "file-23");
    assert.equal(downloads[2].name, "project-attachment");

    globalThis.__projectBinaryApi.readProjectBinaryFile = async () => ({
      name: "corrupt.bin", mimeType: null, dataBase64: "AA==", sizeBytes: 2, sha256: "x",
    });
    await assert.rejects(saveProjectBinaryCopy("owner-project", "file-23"), /size validation/);
    assert.equal(downloads.length, 3);

    // Nonzero unused padding bits are not a canonical encoding, even though
    // permissive decoders may return the same bytes as the valid spelling.
    for (const [encoded, size] of [["Zh==", 1], ["Zm9=", 2]]) {
      globalThis.__projectBinaryApi.readProjectBinaryFile = async () => ({
        name: "corrupt.bin", mimeType: null, dataBase64: encoded,
        sizeBytes: size, sha256: "legacy",
      });
      await assert.rejects(saveProjectBinaryCopy("owner-project", "file-23"), /noncanonical base64 padding/);
      assert.equal(downloads.length, 3, "invalid encoding must not start a download");
    }

    for (const [encoded, expected] of [["Zg==", [102]], ["Zm8=", [102, 111]]]) {
      globalThis.__projectBinaryApi.readProjectBinaryFile = async () => ({
        name: "valid.bin", mimeType: null, dataBase64: encoded,
        sizeBytes: expected.length, sha256: "legacy",
      });
      await saveProjectBinaryCopy("owner-project", "file-23");
      assert.deepEqual([...new Uint8Array(await blobRef.arrayBuffer())], expected);
    }
    assert.equal(downloads.length, 5, "canonical encodings remain downloadable");
  } finally {
    dom.window.HTMLAnchorElement.prototype.click = originalClick;
    globalThis.document = originalDocument;
    globalThis.URL = originalURL;
    globalThis.setTimeout = originalTimer;
    delete globalThis.__projectBinaryApi;
    dom.window.close();
  }
});

test("binary upload rejects malformed declared sizes and inconsistent returned buffers before native writes", async () => {
  let reads = 0;
  let writes = 0;
  globalThis.__projectBinaryApi = {
    addProjectBinaryFile: async () => { writes += 1; return { status: "attached" }; },
  };
  const fakeFile = (size, buffer) => ({
    name: "payload.bin", type: "application/octet-stream", size,
    arrayBuffer: async () => { reads += 1; return buffer; },
  });
  try {
    for (const size of [-1, NaN, Infinity, 1.5, 1_500_001]) {
      await assert.rejects(attachProjectBinaryFile("project", fakeFile(size, new ArrayBuffer(0))), /valid size/);
    }
    assert.equal(reads, 0, "invalid declared sizes must fail before reading");
    await assert.rejects(
      attachProjectBinaryFile("project", fakeFile(1, new ArrayBuffer(1_500_001))),
      /changed during import/,
    );
    await assert.rejects(
      attachProjectBinaryFile("project", fakeFile(4, new ArrayBuffer(3))),
      /changed during import/,
    );
    await assert.rejects(
      attachProjectBinaryFile("project", fakeFile(1, new Uint8Array([7]))),
      /changed during import/,
    );
    assert.equal(reads, 3);
    assert.equal(writes, 0, "invalid uploads must never reach native persistence");
  } finally {
    delete globalThis.__projectBinaryApi;
  }
});

test("failed binary download DOM setup or click immediately revokes its Blob URL", async () => {
  const dom = new JSDOM("<!doctype html><html><body></body></html>", { url: "http://localhost/" });
  const originalDocument = globalThis.document;
  const originalURL = globalThis.URL;
  const originalTimer = globalThis.setTimeout;
  const revoked = [];
  const scheduled = [];
  const originalCreate = dom.window.document.createElement.bind(dom.window.document);
  const originalClick = dom.window.HTMLAnchorElement.prototype.click;
  const originalAppend = dom.window.document.body.appendChild.bind(dom.window.document.body);
  globalThis.document = dom.window.document;
  globalThis.URL = {
    createObjectURL: () => "blob:failed-copy",
    revokeObjectURL: (url) => revoked.push(url),
  };
  globalThis.setTimeout = (fn) => { scheduled.push(fn); return 1; };
  globalThis.__projectBinaryApi = {
    readProjectBinaryFile: async () => ({
      name: "test.bin", mimeType: "application/octet-stream",
      dataBase64: "AQID", sizeBytes: 3, sha256: "fixture",
    }),
  };
  try {
    dom.window.document.createElement = () => { throw new Error("create failed"); };
    await assert.rejects(saveProjectBinaryCopy("project", "file"), /create failed/);
    assert.deepEqual(revoked, ["blob:failed-copy"]);
    assert.equal(scheduled.length, 0);

    dom.window.document.createElement = originalCreate;
    dom.window.document.body.appendChild = () => { throw new Error("append failed"); };
    await assert.rejects(saveProjectBinaryCopy("project", "file"), /append failed/);
    assert.equal(revoked.length, 2);
    assert.equal(dom.window.document.querySelectorAll("a").length, 0);

    dom.window.document.body.appendChild = originalAppend;
    dom.window.HTMLAnchorElement.prototype.click = () => { throw new Error("click failed"); };
    await assert.rejects(saveProjectBinaryCopy("project", "file"), /click failed/);
    assert.equal(revoked.length, 3);
    assert.equal(scheduled.length, 0);
    assert.equal(dom.window.document.querySelectorAll("a").length, 0);

    dom.window.HTMLAnchorElement.prototype.click = () => {};
    await saveProjectBinaryCopy("project", "file");
    assert.equal(revoked.length, 3, "successful click retains URL until handoff");
    assert.equal(scheduled.length, 1);
    scheduled[0]();
    assert.equal(revoked.length, 4);
  } finally {
    dom.window.document.createElement = originalCreate;
    dom.window.document.body.appendChild = originalAppend;
    dom.window.HTMLAnchorElement.prototype.click = originalClick;
    globalThis.document = originalDocument;
    globalThis.URL = originalURL;
    globalThis.setTimeout = originalTimer;
    delete globalThis.__projectBinaryApi;
    dom.window.close();
  }
});

test("corrupt native download metadata is rejected before any browser download", async () => {
  const originalDocument = globalThis.document;
  const originalURL = globalThis.URL;
  let created = 0;
  globalThis.document = {
    createElement() { throw new Error("download must not start"); },
  };
  globalThis.URL = {
    createObjectURL() { created++; throw new Error("download must not start"); },
  };
  const base = {
    name: "নথি.png",
    mimeType: "image/png",
    sizeBytes: 1,
    dataBase64: "AA==",
  };
  const invalid = [
    { ...base, name: null },
    { ...base, name: 42 },
    null,
    { ...base, name: undefined },
    { ...base, mimeType: { toString() { throw new Error("unsafe getter"); } } },
    { ...base, mimeType: "image/png\nX-Evil: true" },
    { ...base, mimeType: "x".repeat(256) },
    { ...base, mimeType: undefined },
    { ...base, dataBase64: null },
    { ...base, dataBase64: 123 },
  ];
  try {
    for (const record of invalid) {
      globalThis.__projectBinaryApi = {
        readProjectBinaryFile: async () => record,
      };
      await assert.rejects(
        saveProjectBinaryCopy("project", "file"),
        /invalid metadata/,
      );
    }
    assert.equal(created, 0);
  } finally {
    globalThis.document = originalDocument;
    globalThis.URL = originalURL;
    delete globalThis.__projectBinaryApi;
  }
});

test.after(async () => {
  await rm(directory, { recursive: true, force: true });
});
