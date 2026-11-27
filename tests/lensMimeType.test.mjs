import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { URL } from "node:url";

// Exercise the actual small pure MIME selector from the TypeScript source,
// rather than reimplementing the allowlist in the test.
const source = readFileSync(new URL("../src/lib/visionMimeType.ts", import.meta.url), "utf8");
const match = source.match(/export function visionMimeType\(file: Pick<File, "name" \| "type">\): "image\/png" \| "image\/jpeg" \| "image\/webp" \| null \{[\s\S]*?\n\}/);
assert.ok(match, "visionMimeType implementation must exist");
const selectorSource = match[0]
  .replace("export function visionMimeType", "function visionMimeType")
  .replace('file: Pick<File, "name" | "type">', "file")
  .replace(': "image/png" | "image/jpeg" | "image/webp" | null', "");
match[0] = selectorSource;
const selector = new Function(
  "SUPPORTED_VISION_IMAGE_TYPES",
  `return (${match[0].replace("file: File", "file")});`,
)(new Set(["image/jpeg", "image/png", "image/webp"]));

test("explicit unsupported image MIME never falls back to a misleading extension", () => {
  for (const type of ["image/svg+xml", "text/html", "application/pdf", "image/gif"]) {
    for (const name of ["photo.png", "photo.jpg", "photo.webp"]) {
      assert.equal(selector({ type, name }), null, `${type} as ${name}`);
    }
  }
});

test("supported declared image MIME remains accepted without extension", () => {
  assert.equal(selector({ type: "image/png", name: "image" }), "image/png");
  assert.equal(selector({ type: "image/jpeg", name: "image.bin" }), "image/jpeg");
  assert.equal(selector({ type: "image/webp", name: "image" }), "image/webp");
});

test("extension fallback is limited to missing or generic MIME", () => {
  assert.equal(selector({ type: "", name: "photo.PNG" }), "image/png");
  assert.equal(selector({ type: "application/octet-stream", name: "photo.JpEg" }), "image/jpeg");
  assert.equal(selector({ type: "", name: "photo.webp" }), "image/webp");
  assert.equal(selector({ type: "application/octet-stream", name: "photo.svg" }), null);
  assert.equal(selector({ type: "image/svg+xml", name: "photo" }), null);
});
