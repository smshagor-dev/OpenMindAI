import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../src/lib/chat.ts", import.meta.url), "utf8");
// Exercise the production allocation function without a DOM/Tauri runtime.
// Remove only the TypeScript parameter and return annotations.
const start = source.indexOf("export function attachmentMedia(");
const end = source.indexOf("\nexport function attachmentMediaCoverage(", start);
assert.ok(start >= 0 && end > start, "production media allocator must exist");
const implementation = source.slice(start, end)
  .replace("export function attachmentMedia(attachments: AttachmentDraft[]): InferenceMediaDraft[]", "function attachmentMedia(attachments)")
  .replace(/: InferenceMediaDraft\[\]/g, "")
  .replace(/: AttachmentDraft\[\]/g, "");
const allocate = vm.runInNewContext(`${implementation}; attachmentMedia`, { MAX_INFERENCE_MEDIA_ITEMS: 4 });

const sourceWith = (name, count) => ({
  name,
  mediaItems: Array.from({ length: count }, (_, i) => ({ name: `${name}-${i + 1}` })),
});

test("two scanned PDFs share the four-frame budget rather than dropping the second PDF", () => {
  const frames = allocate([sourceWith("pdfA", 4), sourceWith("pdfB", 4)]);
  assert.deepEqual(Array.from(frames, x => x.name), ["pdfA-1", "pdfB-1", "pdfA-2", "pdfB-2"]);
});

test("mixed images, video and PDF each receive representation", () => {
  const frames = allocate([sourceWith("image", 1), sourceWith("video", 4), sourceWith("pdf", 4)]);
  assert.deepEqual(Array.from(frames, x => x.name), ["image-1", "video-1", "pdf-1", "video-2"]);
});

test("empty sources and fewer than four frames do not cause duplicates", () => {
  const frames = allocate([sourceWith("empty", 0), sourceWith("pdf", 2)]);
  assert.deepEqual(Array.from(frames, x => x.name), ["pdf-1", "pdf-2"]);
});

test("inference prompt discloses omitted evidence rather than claiming every page was sent", () => {
  assert.match(source, /coverage\.omitted > 0/);
  assert.match(source, /Do not claim to have inspected omitted pages or frames/);
  assert.doesNotMatch(source, /representative page\(s\) sent to Lens/);
});
