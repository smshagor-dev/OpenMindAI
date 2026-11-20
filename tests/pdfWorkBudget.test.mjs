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

test("full selectable budget skips OCR even when earlier scanned pages exist", async () => {
  const fx = fixture(20, (number) => number === 1 ? "" : "selectable ".repeat(2_500));
  const events = [];
  const result = await extractPdfText(file, { onProgress: (event) => events.push(event) });
  assert.deepEqual(fx.visited, [1, 2], "later pages must not be opened after budget fills");
  assert.deepEqual(result.scannedPages, [1]);
  assert.deepEqual(result.unreadPages, [1], "unread scanned page must be disclosed");
  assert.equal(fx.renders.length, 0, "no expensive image rendering after budget fills");
  assert.equal(fx.calls.length, 0, "native OCR must not run when output cannot fit");
  assert.equal(result.visionPages.length, 0);
  assert(result.truncated);
  assert.match(result.warnings.join(" "), /OCR work budget reached/);
  assert(events.some(event => event.phase === "ocr" && event.total === 0));
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


test("OCR stops after the first full text-budget batch and reports unread later pages", async () => {
  const fx = fixture(12, () => "", async (_command, { pages }) =>
    pages.map(page => ({ pageNumber: page.pageNumber, text: "X".repeat(25_000) })));
  const result = await extractPdfText(file);
  assert.equal(fx.calls.length, 1, "no second OCR invocation after text budget is filled");
  assert.deepEqual(fx.calls[0], [1, 2, 3, 4]);
  assert.deepEqual(result.unreadPages, [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
  assert(result.truncated);
  assert.match(result.warnings.join(" "), /PDF text budget reached/);
  assert(result.text.length <= 20_000);
  assert.equal(result.visionPages.length, 0);
  fx.assertClean();
});

test("short OCR output continues into later batches", async () => {
  const fx = fixture(10, () => "", async (_command, { pages }) =>
    pages.map(page => ({ pageNumber: page.pageNumber, text: "short text" })));
  const result = await extractPdfText(file);
  assert.equal(fx.calls.length, 3);
  assert.equal(result.unreadPages.length, 0);
  assert(!result.warnings.some(warning => warning.includes("text budget reached")));
  fx.assertClean();
});

test("OCR deadline expiring inside a batch skips rendering and native dispatch", async () => {
  const fx = fixture(6, (page) => page === 1 ? "Readable selectable text from the first page." : "");
  const originalNow = Date.now;
  let clockReads = 0;
  Date.now = () => [0, 30_000, 61_000][Math.min(clockReads++, 2)];
  try {
    const result = await extractPdfText(file);
    assert.equal(fx.renders.length, 0, "no page should render after deadline");
    assert.equal(fx.calls.length, 0, "native OCR must not start after deadline");
    assert(result.truncated);
    assert(result.unreadPages.length > 0);
    assert.match(result.warnings.join(" "), /time budget reached/);
    fx.assertClean();
  } finally {
    Date.now = originalNow;
  }
});

test("deadline after rendering skips native OCR and preserves readable text", async () => {
  const fx = fixture(6, (page) => page === 1 ? "Readable selectable text from the first page." : "");
  const originalNow = Date.now;
  let clockReads = 0;
  Date.now = () => [0, 0, 0, 61_000][Math.min(clockReads++, 3)];
  try {
    const result = await extractPdfText(file);
    assert.equal(fx.renders.length, 1);
    assert.equal(fx.calls.length, 0, "expired batch must not dispatch native OCR");
    assert.match(result.text, /Readable selectable text/);
    assert(result.truncated);
    assert.match(result.warnings.join(" "), /time budget reached/);
    fx.assertClean();
  } finally {
    Date.now = originalNow;
  }
});

test("invalid decoder page counts reject before page access and always release resources", async () => {
  for (const pageCount of [0, -1, 1.5, NaN, Infinity, -Infinity, Number.MAX_SAFE_INTEGER + 1]) {
    const fx = fixture(pageCount);
    const events = [];
    await assert.rejects(
      extractPdfText(file, { onProgress: event => events.push(event) }),
      /Invalid PDF page count/,
      `invalid page count ${pageCount} must fail`,
    );
    assert.deepEqual(fx.visited, [], "no page reads should occur");
    assert.deepEqual(events, [], "no misleading progress should be emitted");
    assert.deepEqual(fx.calls, [], "native OCR must not be invoked");
    fx.assertClean();
  }
});

test("valid single-page decoder metadata still imports text", async () => {
  const fx = fixture(1, () => "A valid PDF page contains readable selectable evidence.");
  const result = await extractPdfText(file);
  assert.equal(result.pageCount, 1);
  assert.equal(result.pagesRead, 1);
  assert.match(result.text, /readable selectable evidence/);
  assert.deepEqual(fx.visited, [1]);
  fx.assertClean();
});

test.after(async () => {
  globalThis.document = originalDocument;
  delete globalThis.__pdf;
  await rm(directory, { recursive: true, force: true });
});

test("huge native OCR output is bounded before normalization and preserves Unicode boundaries", async () => {
  const huge = " ".repeat(250_000) + "😀 বাংলা evidence " + "Z".repeat(2_000_000);
  const fx = fixture(1, () => "", async () => [{ pageNumber: 1, text: huge }]);
  const result = await extractPdfText(file);
  assert.equal(result.pagesRead, 1);
  assert(result.text.includes("😀 বাংলা evidence"));
  assert(result.text.length <= 20_000);
  assert(!result.text.includes("\uFFFD"), "must not split a surrogate pair");
  assert.equal(result.unreadPages.length, 0);
  fx.assertClean();
});

test("whitespace-only native OCR output is not counted as successfully analyzed", async () => {
  const fx = fixture(1, () => "", async () => [{ pageNumber: 1, text: " ".repeat(500_000) }]);
  await assert.rejects(extractPdfText(file), /No readable PDF text is available/);
  fx.assertClean();
});


test("huge PDF coordinate space never creates an oversized canvas", async () => {
  const fx = fixture(1);
  const pdf = await globalThis.__pdf.task.promise;
  pdf.getPage = async () => ({
    getTextContent: async () => ({ items: [] }),
    getViewport: ({ scale }) => ({ width: 1_000_000 * scale, height: 500_000 * scale }),
    render: ({ canvas }) => {
      assert(canvas.width <= 1440 && canvas.height <= 1440);
      return { promise: Promise.resolve() };
    },
  });
  await extractPdfText(file);
  fx.assertClean();
});

test("invalid PDF viewport is rejected before canvas allocation", async () => {
  const fx = fixture(1);
  const pdf = await globalThis.__pdf.task.promise;
  pdf.getPage = async () => ({
    getTextContent: async () => ({ items: [] }),
    getViewport: () => ({ width: Number.POSITIVE_INFINITY, height: 100 }),
    render: () => { throw new Error("render must not be called"); },
  });
  await assert.rejects(extractPdfText(file), /No readable PDF text/);
  fx.assertClean();
});

test("malformed decoder text items do not discard subsequent readable PDF evidence", async () => {
  const fx = fixture(1);
  const pdf = await globalThis.__pdf.task.promise;
  const originalGetPage = pdf.getPage;
  pdf.getPage = async (pageNumber) => {
    const page = await originalGetPage(pageNumber);
    page.getTextContent = async () => ({
      items: [
        { str: null, hasEOL: true },
        { str: 12345, hasEOL: true },
        { hasEOL: true },
        { str: "Valid selectable evidence from a PDF decoder.", hasEOL: true },
      ],
    });
    return page;
  };
  const result = await extractPdfText(file);
  assert.equal(result.pagesRead, 1);
  assert.match(result.text, /Valid selectable evidence from a PDF decoder/);
  assert.deepEqual(fx.calls, [], "readable text must not trigger OCR");
  assert.deepEqual(fx.renders, [], "readable text must not render a scanned page");
  assert.equal(result.truncated, false);
  fx.assertClean();
});

test("native OCR skips malformed entries and accepts valid results within bounded inspection", async () => {
  const fx = fixture(1, () => "", async () => [
    null, undefined, { pageNumber: "1", text: "invalid" },
    { pageNumber: 1, text: "Valid OCR evidence after malformed entries" },
  ]);
  const result = await extractPdfText(file);
  assert.match(result.text, /Valid OCR evidence after malformed entries/);
  assert.deepEqual(result.unreadPages, []);
  fx.assertClean();
});

test("native OCR response inspection stops at 16 entries per rendered page", async () => {
  const entries = Array.from({ length: 100_000 }, () => ({ pageNumber: -1, text: "invalid" }));
  Object.defineProperty(entries, 16, {
    get() { throw new Error("OCR must not inspect beyond its response budget"); },
  });
  const fx = fixture(1, () => "Readable selectable text before the scanned OCR page.");
  const pdf = await globalThis.__pdf.task.promise;
  const originalGetPage = pdf.getPage;
  pdf.numPages = 2;
  pdf.getPage = async (number) => {
    const page = await originalGetPage(number);
    page.getTextContent = async () => ({ items: [
      { str: number === 1 ? "Readable selectable text before the scanned OCR page." : "", hasEOL: true },
    ] });
    return page;
  };
  globalThis.__pdf.ocr = async () => entries;
  const result = await extractPdfText(file);
  assert.match(result.text, /Readable selectable text/);
  assert.deepEqual(result.unreadPages, [2]);
  assert.equal(result.visionPages.length, 0);
  fx.assertClean();
});
