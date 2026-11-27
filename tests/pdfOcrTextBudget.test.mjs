import { URL } from "node:url";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// Structural regression: the early budget guard must precede rendering and
// native OCR. A runtime fixture with a scanned PDF is still required.
const source = readFileSync(new URL("../src/lib/pdf.ts", import.meta.url), "utf8");
const loop = source.indexOf("for (let offset = 0; offset < ocrPages.length;");
const guard = source.indexOf("if (extractedChars >= MAX_EXTRACTED_CHARS)", loop);
const render = source.indexOf("await renderPdfPage(", loop);
const invoke = source.indexOf('invoke<PdfOcrPageResult[]>("ocr_pdf_pages"', loop);

test("exhausted selectable-text budget stops OCR before rendering", () => {
  assert.ok(loop >= 0 && guard > loop);
  assert.ok(render > guard, "guard must run before page rendering");
  assert.ok(invoke > guard, "guard must run before native OCR");
  assert.match(source.slice(guard, render), /stoppedForTextBudget = true;\s*break;/);
});

test("unprocessed OCR pages remain reported as unread", () => {
  assert.match(source, /const unreadPages = new Set\(ocrPages\)/);
  assert.match(source, /const missing = \[\.\.\.unreadPages, \.\.\.deferred\]/);
  assert.match(source, /if \(missing\.length\) visionPages\.length = 0/);
});
