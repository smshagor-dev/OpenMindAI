import test from "node:test";
import assert from "node:assert/strict";
import { visionMimeType } from "../src/lib/visionMimeType.ts";

test("explicit unsupported MIME cannot be overridden by a misleading extension", () => {
  for (const type of ["image/svg+xml", "text/html", "application/pdf", "audio/mpeg"]) {
    assert.equal(visionMimeType({ name: "diagram.png", type }), null);
  }
});

test("supported declared MIME wins over a conflicting extension", () => {
  assert.equal(visionMimeType({ name: "photo.pdf", type: "image/jpeg" }), "image/jpeg");
  assert.equal(visionMimeType({ name: "photo.jpg", type: "IMAGE/PNG" }), "image/png");
});

test("only empty or generic MIME permits extension inference", () => {
  assert.equal(visionMimeType({ name: "PHOTO.JPEG", type: "" }), "image/jpeg");
  assert.equal(visionMimeType({ name: "photo.webp", type: "application/octet-stream" }), "image/webp");
  assert.equal(visionMimeType({ name: "photo.gif", type: "" }), null);
  assert.equal(visionMimeType({ name: "photo.png", type: "image/gif" }), null);
});
