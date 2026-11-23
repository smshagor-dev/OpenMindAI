import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

// Extract the exported helper directly from production source; no duplicate
// formatting implementation in the test fixture.
const source = readFileSync(new URL("../src/lib/format.ts", import.meta.url), "utf8");
const match = source.match(/export function formatBytes\(value\?: number \| null\) \{[\s\S]*?\n\}/);
assert.ok(match, "formatBytes implementation must be present");
const formatBytes = new Function(match[0].replace(
  "export function formatBytes(value?: number | null)", "return function formatBytes(value)"
))();

test("formatBytes preserves existing small and absent values", () => {
  assert.equal(formatBytes(undefined), undefined);
  assert.equal(formatBytes(null), undefined);
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(1), "1 B");
  assert.equal(formatBytes(1023), "1023 B");
  assert.equal(formatBytes(1024), "1.0 KB");
  assert.equal(formatBytes(1024 ** 4 - 1024 ** 3), "1023.0 GB");
});

test("formatBytes advances across TB, PB and EB boundaries", () => {
  assert.equal(formatBytes(1024 ** 4), "1.0 TB");
  assert.equal(formatBytes(1024 ** 5 - 1024 ** 4), "1023.0 TB");
  assert.equal(formatBytes(1024 ** 5), "1.0 PB");
  assert.equal(formatBytes(1024 ** 6 - 1024 ** 5), "1023.0 PB");
  assert.equal(formatBytes(1024 ** 6), "1.0 EB");
  assert.equal(formatBytes(2 * 1024 ** 6), "2.0 EB");
});
