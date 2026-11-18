import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// Source-level guard: native OCR must not run when selectable PDF text
// has already consumed the model context budget. Runtime PDF fixtures
// remain necessary for end-to-end validation.
const source = readFileSync(new URL("../src/lib/pdf.ts", import.meta.url), "utf8");

test("PDF OCR selection skips scanned pages after selectable text fills the budget", () => {
  assert.match(source, /const ocrPages = selectableChars >= MAX_EXTRACTED_CHARS\\s*\\? \\[\\]\\s*:\\s*selectOcrPages\\(scannedPages, MAX_OCR_PAGES\\)/);
  assert.match(source, /const deferred = scannedPages\\.filter\\(pageNumber => !ocrPageSet\\.has\\(pageNumber\\)\\)/);
  assert.match(source, /if \\(deferred\\.length \\|\\| stoppedForTime \\|\\| stoppedForTextBudget\\) truncated = true/);
  assert.match(source, /if \\(missing\\.length\\) visionPages\\.length = 0/);
});
