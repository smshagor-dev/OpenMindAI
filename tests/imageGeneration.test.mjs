import assert from "node:assert/strict";
import test from "node:test";
import {
  imageRendererPrompt,
  isImageGenerationIntent,
  plainImageRequest,
} from "../src/lib/imageGeneration.ts";

test("recognizes natural image-generation requests", () => {
  for (const prompt of [
    "Create an image of a lunar base",
    "Generate me a photo of Dhaka at night",
    "Make a new picture for the launch",
    "Design a professional banner",
    "Draw an illustration of a robot",
  ]) {
    assert.equal(isImageGenerationIntent(prompt), true, prompt);
  }
  assert.equal(isImageGenerationIntent("draw a conclusion from this report"), false);
});

test("removes persisted image-mode instructions from the renderer fallback", () => {
  const persisted =
    "[Mode: Image Creation]\nCreate a concise production-quality prompt.\n\nA red fox in snow";
  assert.equal(plainImageRequest(persisted), "A red fox in snow");
});

test("falls back to the user's request when the text model refuses image generation", () => {
  const persisted = "[Mode: Image Creation]\nRenderer instructions\n\nA cinematic Mars rover";
  assert.equal(
    imageRendererPrompt("I'm sorry, but I can't generate images.", persisted),
    "A cinematic Mars rover",
  );
});

test("keeps a successful prompt enhancement", () => {
  assert.equal(
    imageRendererPrompt("A detailed studio portrait, soft rim light", "Create an image"),
    "A detailed studio portrait, soft rim light",
  );
});
