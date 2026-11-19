import test from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const dir = await mkdtemp(join(tmpdir(), "attachment-mime-"));
let classifyAttachmentKind;
try {
  const output = join(dir, "attachment-kind.mjs");
  await build({
    entryPoints: ["src/lib/attachmentKind.ts"],
    outfile: output,
    bundle: true,
    platform: "node",
    format: "esm",
  });
  ({ classifyAttachmentKind } = await import(pathToFileURL(output).href));
} finally {
  await rm(dir, { recursive: true, force: true });
}

test("supported PDF MIME overrides misleading image suffix", () => {
  assert.equal(classifyAttachmentKind({ name: "report.png", type: "application/pdf" }), "pdf");
  assert.equal(classifyAttachmentKind({ name: "report.mp3", type: "application/pdf" }), "pdf");
});

test("supported image, audio and video MIME override conflicting extensions", () => {
  assert.equal(classifyAttachmentKind({ name: "photo.pdf", type: "image/jpeg" }), "image");
  assert.equal(classifyAttachmentKind({ name: "voice.pdf", type: "audio/wav" }), "audio");
  assert.equal(classifyAttachmentKind({ name: "clip.jpg", type: "video/mp4" }), "video");
  assert.equal(classifyAttachmentKind({ name: "notes.pdf", type: "text/plain" }), "text");
});

test("extension fallback remains available for missing and generic MIME", () => {
  assert.equal(classifyAttachmentKind({ name: "scan.PDF", type: "" }), "pdf");
  assert.equal(classifyAttachmentKind({ name: "clip.webm", type: "application/octet-stream" }), "video");
  assert.equal(classifyAttachmentKind({ name: "readme.md", type: "" }), "text");
  assert.equal(classifyAttachmentKind({ name: "unknown.bin", type: "" }), "binary");
});

test("unsupported MIME does not become a supported MIME solely by its value", () => {
  assert.equal(classifyAttachmentKind({ name: "blob.bin", type: "application/x-custom" }), "binary");
  assert.equal(classifyAttachmentKind({ name: "blob.bin", type: "image/svg+xml" }), "image");
});
