import test from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const directory = await mkdtemp(join(tmpdir(), "pdf-budget-"));
const output = join(directory, "pdf.mjs");
await build({
  entryPoints: ["src/lib/pdf.ts"],
  outfile: output,
  bundle: true,
  format: "esm",
  platform: "node",
  plugins: [{
    name: "local-pdf-mocks",
    setup(builder) {
      builder.onResolve({ filter: /^pdfjs-dist$/ }, () => ({ path: "pdfjs", namespace: "fixture" }));
      builder.onResolve({ filter: /pdf\.worker.*\?url$/ }, () => ({ path: "worker", namespace: "fixture" }));
      builder.onResolve({ filter: /^@tauri-apps\/api\/core$/ }, () => ({ path: "native", namespace: "fixture" }));
      builder.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => ({
        contents: ({
          pdfjs: "export const GlobalWorkerOptions={}; export const getDocument=()=>globalThis.__pdf.task;",
          worker: 'export default "fake-worker";',
          native: "export const invoke=(...args)=>globalThis.__pdf.ocr(...args);",
        })[path],
      }));
    },
  }],
});

const { extractPdfText } = await import(pathToFileURL(output));
const originalDocument = globalThis.document;
const canvasDimensions = [];
globalThis.document = {
  createElement: () => {
    const canvas = {
      width: 0,
      height: 0,
      getContext: () => ({ fillRect() {}, fillStyle: "" }),
      toDataURL: () => "data:image/jpeg;base64,AA==",
    };
    canvasDimensions.push(canvas);
    return canvas;
  },
};

const file = { name: "scan.pdf", size: 10, arrayBuffer: async () => new ArrayBuffer(10) };

function fixture(pages, textForPage = () => "", ocr = async (_command, { pages }) =>
  pages.map((page) => ({ pageNumber: page.pageNumber, text: `OCR PAGE ${page.pageNumber}` }))) {
  const visited = [];
  const renders = [];
  const calls = [];
  let destroyed = false;
  globalThis.__pdf = {
    ocr: async (...args) => {
      calls.push(args[1].pages.map((page) => page.pageNumber));
      return ocr(...args);
    },
    task: {
      promise: Promise.resolve({
        numPages: pages,
        getPage: async (pageNumber) => {
          visited.push(pageNumber);
          return {
            getTextContent: async () => ({
              items: [{ str: textForPage(pageNumber), hasEOL: true }],
            }),
            getViewport: () => ({ width: 100, height: 100 }),
            render: () => {
              renders.push(pageNumber);
              return { promise: Promise.resolve() };
            },
          };
        },
      }),
      destroy: async () => { destroyed = true; },
    },
  };
  return { visited, renders, calls, assertClean: () => assert(destroyed, "PDF resources must be destroyed") };
}

test("250 image-only pages OCR at most 16 sampled pages rather than all 250", async () => {
  const fx = fixture(250);
  const events = [];
  const result = await extractPdfText(file, { onProgress: (step) => events.push(step) });
  assert.equal(result.pageCount, 250);
  assert.equal(result.pagesRead, 250);
  assert.equal(result.scannedPages.length, 250);
  const ocrNumbers = fx.calls.flat();
  assert.equal(ocrNumbers.length, 16);
  assert.equal(new Set(ocrNumbers).size, 16);
  assert.equal(fx.calls.length, 4);
  assert.equal(fx.renders.length, 16);
  assert(ocrNumbers.includes(1) && ocrNumbers.includes(8), "first pages retain priority");
  assert(ocrNumbers.includes(250), "late representative page must not be silently omitted");
  assert(ocrNumbers.some((number) => number > 50 && number < 200), "sample interior evidence");
  assert.equal(result.unreadPages.length, 234);
  assert.match(result.warnings.join(" "), /234 scanned page/);
  assert.match(result.warnings.join(" "), /Split the PDF/);
  assert.equal(result.visionPages.length, 0, "partial analysis must not advertise full Lens coverage");
  assert(result.truncated);
  assert.match(result.text, /OCR PAGE 1/);
  assert(events.some((step) => step.phase === "reading" && step.completed === 250));
  assert(events.some((step) => step.phase === "ocr" && step.completed === 16));
  assert(canvasDimensions.every((canvas) => canvas.width === 0 && canvas.height === 0));
  fx.assertClean();
});

test("large selectable page ends extraction before opening later pages", async () => {
  const fx = fixture(250, (number) => number === 1 ? "selectable ".repeat(2_500) : "");
  const result = await extractPdfText(file);
  assert.deepEqual(fx.visited, [1]);
  assert.equal(result.pagesRead, 1);
  assert(result.truncated);
  assert.equal(result.scannedPages.length, 0);
  assert.equal(result.text.length <= 20_000, true);
  assert.equal(fx.renders.length, 0);
  assert.equal(fx.calls.length, 0);
  fx.assertClean();
});

test("explicit cancellation stops further page reads, releases decoder, and rejects import", async () => {
  const fx = fixture(250);
  const abort = new globalThis.AbortController();
  await assert.rejects(
    extractPdfText(file, {
      signal: abort.signal,
      onProgress: (step) => {
        if (step.phase === "reading" && step.completed === 3) abort.abort();
      },
    }),
    /PDF import cancelled/,
  );
  assert.deepEqual(fx.visited, [1, 2, 3]);
  assert.equal(fx.calls.length, 0);
  fx.assertClean();
});

test("complete small scanned document preserves OCR and representative vision behavior", async () => {
  const fx = fixture(6);
  const result = await extractPdfText(file);
  assert.equal(fx.renders.length, 6);
  assert.equal(fx.calls.length, 2);
  assert.equal(result.unreadPages.length, 0);
  assert.equal(result.visionPages.length, 4);
  assert.equal(result.warnings.length, 0);
  assert(!result.truncated);
  fx.assertClean();
});

test.after(async () => {
  globalThis.document = originalDocument;
  delete globalThis.__pdf;
  await rm(directory, { recursive: true, force: true });
});
