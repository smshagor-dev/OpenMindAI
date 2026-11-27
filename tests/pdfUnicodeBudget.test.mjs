import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { transformSync } from "esbuild";

// Exercise the exact production helper in isolation without a PDF worker.
const source = readFileSync(new URL("../src/lib/pdf.ts", import.meta.url), "utf8");
const match = source.match(/function slicePdfText\(text: string, maxUnits: number\): string \{[\s\S]*?\n\}/);
assert.ok(match, "production PDF budget helper must exist");
const js = transformSync(match[0] + "\nexport { slicePdfText };", { loader: "ts", format: "cjs" }).code;
const module = { exports: {} };
new Function("module", "exports", js)(module, module.exports);
const { slicePdfText } = module.exports;

test("PDF text budget never emits a dangling high surrogate", () => {
  assert.equal(slicePdfText("a😀b", 2), "a");
  assert.equal(slicePdfText("a😀b", 3), "a😀");
  assert.equal(slicePdfText("a😀b", 4), "a😀b");
});

test("PDF text budget handles zero, negative, and oversized limits", () => {
  assert.equal(slicePdfText("😀", 1), "");
  assert.equal(slicePdfText("😀", 0), "");
  assert.equal(slicePdfText("abc", -1), "");
  assert.equal(slicePdfText("😀abc", 99), "😀abc");
});
