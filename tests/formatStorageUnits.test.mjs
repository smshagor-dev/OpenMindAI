import test from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const directory = await mkdtemp(join(tmpdir(), "format-storage-units-"));
const output = join(directory, "format.mjs");
await build({
  entryPoints: ["src/lib/format.ts"],
  outfile: output,
  bundle: true,
  format: "esm",
  platform: "node",
});
const { formatBytes } = await import(pathToFileURL(output).href);
test.after(async () => { await rm(directory, { recursive: true, force: true }); });

test("formats byte sizes through petabytes and exabytes without clamping at TB", () => {
  assert.equal(formatBytes(1024 ** 4), "1.0 TB");
  assert.equal(formatBytes(1024 ** 5), "1.0 PB");
  assert.equal(formatBytes(1.5 * 1024 ** 5), "1.5 PB");
  assert.equal(formatBytes(1024 ** 6), "1.0 EB");
  assert.equal(formatBytes(2 * 1024 ** 6), "2.0 EB");
});

test("preserves existing zero, missing and ordinary size formatting", () => {
  assert.equal(formatBytes(undefined), undefined);
  assert.equal(formatBytes(null), undefined);
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(1023), "1023 B");
  assert.equal(formatBytes(1024), "1.0 KB");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(1024 ** 2), "1.0 MB");
});
