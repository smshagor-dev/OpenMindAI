import assert from "node:assert/strict";
import test from "node:test";
import {
  DIRECT_IMAGE_MESSAGE,
  imageRendererPrompt,
  isImageGenerationIntent,
  persistDirectImageRequest,
  plainImageRequest,
} from "../src/lib/imageGeneration.ts";

test("recognizes natural image-generation requests in English, Bangla, and Russian", () => {
  for (const prompt of [
    "Create an image of a lunar base",
    "Generate me a photo of Dhaka at night",
    "Make a new picture for the launch",
    "Design a professional banner",
    "Draw an illustration of a robot",
    "Draw a cat",
    "Paint a beautiful sunset",
    "একটা ছবি তৈরি করো",
    "আমার জন্য লোগো বানাও",
    "Создай изображение кошки",
    "Нарисуй кота",
  ]) {
    assert.equal(isImageGenerationIntent(prompt), true, prompt);
  }
  for (const prompt of [
    "draw a conclusion from this report",
    "Explain how image generation works",
    "Tell me about the history of a logo",
  ]) {
    assert.equal(isImageGenerationIntent(prompt), false, prompt);
  }
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
  assert.equal(imageRendererPrompt(DIRECT_IMAGE_MESSAGE, persisted), "A cinematic Mars rover");
});

test("keeps a successful legacy prompt enhancement", () => {
  assert.equal(
    imageRendererPrompt("A detailed studio portrait, soft rim light", "Create an image"),
    "A detailed studio portrait, soft rim light",
  );
});

test("direct image request persists history without invoking chat inference", async () => {
  const calls = [];
  const persistedContent = "[Mode: Image Creation]\nRenderer instructions\n\nA fox in snow";
  const assistant = await persistDirectImageRequest("conversation-1", persistedContent, {
    addUserMessage: async (...args) => { calls.push(["user", ...args]); },
    createStreamingAssistantMessage: async (...args) => {
      calls.push(["assistant", ...args]);
      return { id: "message-1" };
    },
    appendMessageChunk: async (...args) => { calls.push(["chunk", ...args]); },
    completeMessage: async (...args) => { calls.push(["complete", ...args]); },
  });
  assert.equal(assistant.id, "message-1");
  assert.deepEqual(calls, [
    ["user", "conversation-1", persistedContent],
    ["assistant", "conversation-1"],
    ["chunk", "message-1", DIRECT_IMAGE_MESSAGE],
    ["complete", "message-1", "completed"],
  ]);
});

test("persist error marks the generated assistant turn failed", async () => {
  const calls = [];
  await assert.rejects(
    persistDirectImageRequest("conversation-1", "A cat", {
      addUserMessage: async () => {},
      createStreamingAssistantMessage: async () => ({ id: "message-2" }),
      appendMessageChunk: async () => { throw new Error("disk full"); },
      completeMessage: async (...args) => { calls.push(args); },
    }),
    /disk full/,
  );
  assert.deepEqual(calls, [["message-2", "failed"]]);
});
