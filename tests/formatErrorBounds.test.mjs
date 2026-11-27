import test from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const directory = await mkdtemp(join(tmpdir(), "format-error-bound-"));
const outfile = join(directory, "format.mjs");
await build({ entryPoints: ["src/lib/format.ts"], outfile, bundle: true, format: "esm", platform: "node" });
const { formatError } = await import(pathToFileURL(outfile).href);

test("short application errors retain their original text", () => {
  assert.equal(formatError("offline"), "offline");
  assert.equal(formatError(new Error("network failed")), "network failed");
  assert.equal(formatError({ reason: "permission denied" }), "permission denied");
});

test("long string, Error and JSON fallback messages are bounded", () => {
  for (const error of ["x".repeat(100_000), new Error("y".repeat(100_000)), { detail: "z".repeat(100_000) }]) {
    const text = formatError(error);
    assert.ok(text.length <= 2_048);
    assert.ok(text.endsWith("…"));
  }
});

test("truncation never creates a lone high surrogate at the boundary", () => {
  const result = formatError("a".repeat(2_046) + "😀" + "b".repeat(20));
  assert.ok(result.length <= 2_048);
  assert.ok(!/[\uD800-\uDBFF]$/.test(result.slice(0, -1)));
  assert.ok(result.endsWith("…"));
});

test.after(async () => {
  await rm(directory, { recursive: true, force: true });
});
