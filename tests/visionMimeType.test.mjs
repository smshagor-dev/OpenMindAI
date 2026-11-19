import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

// Exercise the production MIME classifier without loading Tauri or browser globals.
const source = readFileSync(new URL("../src/lib/chat.ts", import.meta.url), "utf8");
const match = source.match(/function visionMimeType\(file: File\) \{([\s\S]*?)\n\}/);
assert.ok(match, "visionMimeType must remain discoverable");
const classify = vm.runInNewContext(
  `(file) => { const SUPPORTED_VISION_IMAGE_TYPES = new Set(["image/jpeg", "image/png", "image/webp"]); ${match[1]} }`,
);
const file = (name, type) => ({ name, type });

test("explicit unsupported MIME cannot be disguised by a supported extension", () => {
  for (const type of ["image/svg+xml", "text/html", "application/pdf", "image/gif"]) {
    assert.equal(classify(file("diagram.png", type)), null, type);
    assert.equal(classify(file("photo.JPG", type)), null, type);
    assert.equal(classify(file("picture.webp", type)), null, type);
  }
});

test("known supported MIME is authoritative even if extension differs", () => {
  assert.equal(classify(file("image.bin", "image/png")), "image/png");
  assert.equal(classify(file("image.png", "IMAGE/JPEG")), "image/jpeg");
});

test("missing or generic MIME permits supported filename fallback only", () => {
  for (const type of ["", "application/octet-stream"]) {
    assert.equal(classify(file("photo.JPEG", type)), "image/jpeg");
    assert.equal(classify(file("photo.jpg", type)), "image/jpeg");
    assert.equal(classify(file("photo.png", type)), "image/png");
    assert.equal(classify(file("photo.webp", type)), "image/webp");
    assert.equal(classify(file("photo.svg", type)), null);
    assert.equal(classify(file("photo.png.exe", type)), null);
  }
});
