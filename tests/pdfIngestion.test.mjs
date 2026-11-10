import test from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const dir = await mkdtemp(join(tmpdir(), "openmind-pdf-"));
const output = join(dir, "pdf.mjs");
await build({ entryPoints: ["src/lib/chat.ts"], outfile: output, bundle: true, format: "esm", platform: "node", plugins: [{
  name: "pdf-boundaries", setup(b) {
    b.onResolve({ filter: /^pdfjs-dist$/ }, () => ({ path: "pdfjs", namespace: "fixture" }));
    b.onResolve({ filter: /pdf\.worker.*\?url$/ }, () => ({ path: "worker", namespace: "fixture" }));
    b.onResolve({ filter: /^@tauri-apps\/api\/core$/ }, () => ({ path: "native", namespace: "fixture" }));
    b.onResolve({ filter: /^\.\/media$/ }, () => ({ path: "media", namespace: "fixture" }));
    b.onLoad({ filter: /.*/, namespace: "fixture" }, ({path}) => ({ contents: {
      pdfjs: "export const GlobalWorkerOptions={};export const getDocument=()=>globalThis.__pdf.task;",
      worker: 'export default "fixture-worker";',
      native: "export const invoke=(...args)=>globalThis.__pdf.ocr(...args);",
      media: "export const analyzeVideoFile=()=>{},transcribeAudioFile=()=>{};",
    }[path], loader: "js" }));
  },
}] });
const { readAttachment, inferChatMode, attachmentMedia } = await import(pathToFileURL(output));
const file = { name: "mixed.pdf", type: "application/pdf", size: 10, arrayBuffer: async () => new ArrayBuffer(10) };
const originalDocument = globalThis.document;
globalThis.document = { createElement: () => ({ getContext: () => ({ fillRect() {} }), toDataURL: () => "data:image/jpeg;base64,AA==" }) };
function fixture(texts, ocr, renderFailure = 0) {
  let destroyed = false;
  globalThis.__pdf = {
    ocr,
    task: { promise: Promise.resolve({ numPages: texts.length, getPage: async n => ({
      getTextContent: async () => ({ items: [{ str: texts[n - 1], hasEOL: true }] }),
      getViewport: () => ({ width: 100, height: 100 }),
      render: () => ({ promise: n === renderFailure ? Promise.reject(Error("render unavailable")) : Promise.resolve() }),
    }) }), destroy: async () => { destroyed = true; } },
  };
  return () => assert(destroyed, "parser resources released");
}
const readable = "Selectable text from the original page must remain available for text chat.";
test("mixed PDF survives missing Lens and routes through a text model with a visible warning", async () => {
  const cleanup = fixture([readable, ""], async () => { throw Error("Lens not installed"); });
  const result = await readAttachment(file);
  assert.match(result.contentPreview, /Selectable text/);
  assert.match(result.contentPreview, /Page 1/);
  assert.match(result.warning, /pages 2/);
  assert.match(result.warning, /Install or repair OpenMindAI Lens/);
  assert.equal(inferChatMode("summarize", [result]), "pdf");
  assert.deepEqual(attachmentMedia([result]), []);
  cleanup();
});
test("successful OCR batches survive a later batch failure and identify only unread pages", async () => {
  let calls = 0;
  const cleanup = fixture([readable, "", "", "", "", ""], async (_command, { pages }) => {
    if (++calls === 2) throw Error("runtime failed");
    return pages.map(page => ({ pageNumber: page.pageNumber, text: `OCR text on page ${page.pageNumber}` }));
  });
  const result = await readAttachment(file);
  assert.match(result.contentPreview, /OCR text on page 2/);
  assert.match(result.warning, /pages 6\./);
  assert.doesNotMatch(result.warning, /pages 2/);
  assert.equal(result.mediaItems.length, 0);
  cleanup();
});
test("render errors and omitted/empty OCR results remain page-level limitations", async () => {
  const cleanup = fixture([readable, "caption", ""], async () => [], 3);
  const result = await readAttachment(file);
  assert.match(result.contentPreview, /caption/);
  assert.match(result.warning, /pages 2, 3/);
  cleanup();
});
test("entirely scanned PDF still reports the Lens requirement when no text is usable", async () => {
  const cleanup = fixture(["", ""], async () => { throw Error("Lens missing"); });
  await assert.rejects(readAttachment(file), /No readable PDF text.*Install or repair OpenMindAI Lens/);
  cleanup();
});
test("complete OCR retains the existing visual path without incomplete warnings", async () => {
  const cleanup = fixture([readable, ""], async (_command, { pages }) => pages.map(page => ({ pageNumber: page.pageNumber, text: "Scanned page content" })));
  const result = await readAttachment(file);
  assert.equal(result.warning, undefined);
  assert.match(result.contentPreview, /Scanned page content/);
  assert.equal(inferChatMode("summarize", [result]), "vision");
  cleanup();
});
test.after(async () => { globalThis.document = originalDocument; delete globalThis.__pdf; await rm(dir, { recursive: true, force: true }); });
