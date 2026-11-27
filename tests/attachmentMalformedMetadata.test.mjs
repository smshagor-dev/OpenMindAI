import test from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const dir = await mkdtemp(join(tmpdir(), "attachment-metadata-"));
let classifyAttachmentKind;
try {
  const output = join(dir, "kind.mjs");
  await build({ entryPoints: ["src/lib/attachmentKind.ts"], outfile: output, bundle: true, platform: "node", format: "esm" });
  ({ classifyAttachmentKind } = await import(pathToFileURL(output).href));
} finally {
  await rm(dir, { recursive: true, force: true });
}

test("missing or malformed MIME falls back to a valid filename", () => {
  assert.equal(classifyAttachmentKind({ name: "report.pdf", type: null }), "pdf");
  assert.equal(classifyAttachmentKind({ name: "voice.mp3", type: 42 }), "audio");
  assert.equal(classifyAttachmentKind({ name: "notes.md", type: { toLowerCase() { throw Error("coercion"); } } }), "text");
});

test("missing or malformed filename does not crash supported MIME routing", () => {
  assert.equal(classifyAttachmentKind({ name: null, type: "image/png" }), "image");
  assert.equal(classifyAttachmentKind({ name: 123, type: "application/pdf" }), "pdf");
  assert.equal(classifyAttachmentKind({ name: Symbol("bad"), type: "" }), "binary");
});

test("throwing metadata accessors are isolated from one another", () => {
  const badType = { name: "clip.webm", get type() { throw Error("type unavailable"); } };
  const badName = { type: "audio/wav", get name() { throw Error("name unavailable"); } };
  const both = { get type() { throw Error("type"); }, get name() { throw Error("name"); } };
  assert.equal(classifyAttachmentKind(badType), "video");
  assert.equal(classifyAttachmentKind(badName), "audio");
  assert.equal(classifyAttachmentKind(both), "binary");
});

test("normal supported MIME still outranks a conflicting filename", () => {
  assert.equal(classifyAttachmentKind({ name: "image.png", type: "application/pdf" }), "pdf");
  assert.equal(classifyAttachmentKind({ name: "document.pdf", type: "text/plain" }), "text");
  assert.equal(classifyAttachmentKind({ name: "plain.txt", type: "" }), "text");
});
