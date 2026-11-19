import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/lib/pdf.ts", import.meta.url), "utf8");
const start = source.indexOf("async function selectablePageText(");
const end = source.indexOf("\nasync function renderPdfPage(", start);
assert.ok(start >= 0 && end > start, "production extraction function exists");
const functionSource = source.slice(start, end).replace("page: PDFPageProxy", "page");
const selectablePageText = new Function(
  "MAX_EXTRACTED_CHARS",
  "return (" + functionSource.replace("async function selectablePageText", "async function") + ");",
)(20_000);
function page(items) {
  return { getTextContent: async () => ({ items }) };
}
test("single oversized item is bounded before normalization", async () => {
  const text = await selectablePageText(page([{ str: "A".repeat(2_000_000), hasEOL: false }]));
  assert.equal(text.length, 20_000);
  assert.equal(text, "A".repeat(20_000));
});
test("ordinary text preserves line boundaries and whitespace", async () => {
  const text = await selectablePageText(page([
    { str: "Hello   world", hasEOL: true },
    { str: "Second\tline", hasEOL: false },
  ]));
  assert.equal(text, "Hello world\nSecond line");
});
test("many text items cannot exceed page budget", async () => {
  const items = Array.from({ length: 200 }, (_, index) => ({
    str: "x".repeat(300),
    hasEOL: index % 3 === 0,
  }));
  const text = await selectablePageText(page(items));
  assert.ok(text.length <= 20_000);
  assert.ok(text.length > 0);
});
