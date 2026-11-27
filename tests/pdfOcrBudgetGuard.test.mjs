import { URL } from "node:url";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// Source-level guard: runtime PDF fixtures still needed for OCR invocation counts.
const source = readFileSync(new URL("../src/lib/pdf.ts", import.meta.url), "utf8");
const loop = source.slice(source.indexOf("for (let offset = 0; offset < ocrPages.length;"));
test("OCR loop checks text budget before rendering any page", () => {
  assert.ok(loop.startsWith("for (let offset = 0;"));
  const guard = loop.indexOf("if (extractedChars >= MAX_EXTRACTED_CHARS)");
  const render = loop.indexOf("renderPdfPage(");
  const invoke = loop.indexOf('invoke<PdfOcrPageResult[]>("ocr_pdf_pages"');
  assert.ok(guard >= 0 && guard < render && render < invoke);
  assert.match(loop.slice(guard, render), /stoppedForTextBudget = true;\s*break;/);
});
test("unprocessed OCR pages are disclosed rather than marked analyzed", () => {
  assert.match(source, /const unreadPages = new Set\(ocrPages\)/);
  assert.match(source, /const missing = \[\.\.\.unreadPages, \.\.\.deferred\]/);
  assert.match(source, /if \(missing\.length\) visionPages\.length = 0/);
  assert.match(source, /stoppedForTextBudget \? \[/);
});

test("PDF OCR selection skips scanned pages after selectable text fills the budget", () => {
  assert.match(source, /const ocrPages = selectableChars >= MAX_EXTRACTED_CHARS\s*\? \[\]\s*:\s*selectOcrPages\(scannedPages, MAX_OCR_PAGES\)/);
  assert.match(source, /const deferred = scannedPages\.filter\(pageNumber => !ocrPageSet\.has\(pageNumber\)\)/);
  assert.match(source, /if \(deferred\.length \|\| stoppedForTime \|\| stoppedForTextBudget\) truncated = true/);
  assert.match(source, /if \(missing\.length\) visionPages\.length = 0/);
});
