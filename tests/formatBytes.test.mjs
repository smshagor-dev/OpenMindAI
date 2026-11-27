import test from "node:test";
import assert from "node:assert/strict";
import { formatBytes } from "../src/lib/format.ts";

test("invalid byte metadata is never displayed as a real storage size", () => {
  for (const value of [NaN, Infinity, -Infinity, -1, -0.1]) {
    assert.equal(formatBytes(value), "Unknown size");
  }
});

test("missing and zero sizes retain their existing behavior", () => {
  assert.equal(formatBytes(), undefined);
  assert.equal(formatBytes(null), undefined);
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(-0), "0 B");
});

test("finite valid byte sizes retain unit conversion and upper-unit bounds", () => {
  assert.equal(formatBytes(1), "1 B");
  assert.equal(formatBytes(1023), "1023 B");
  assert.equal(formatBytes(1024), "1.0 KB");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(1024 ** 4), "1.0 TB");
  assert.equal(formatBytes(1024 ** 5), "1.0 PB");
  assert.equal(formatBytes(1024 ** 6), "1.0 EB");
  assert.equal(formatBytes(1024 ** 7), "1024.0 EB");
});

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
