import test from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const dir = await mkdtemp(join(tmpdir(), "openmind-routing-"));
const output = join(dir, "routing.mjs");
await build({ entryPoints: ["src/lib/chat.ts"], outfile: output, bundle: true, format: "esm", platform: "node", plugins: [{
  name: "pdf-boundaries", setup(b) {
    b.onResolve({ filter: /^pdfjs-dist$/ }, () => ({ path: "pdfjs", namespace: "fixture" }));
    b.onResolve({ filter: /pdf\.worker.*\?url$/ }, () => ({ path: "worker", namespace: "fixture" }));
    b.onResolve({ filter: /^@tauri-apps\/api\/core$/ }, () => ({ path: "native", namespace: "fixture" }));
    b.onResolve({ filter: /^\.\/media$/ }, () => ({ path: "media", namespace: "fixture" }));
    b.onLoad({ filter: /.*/, namespace: "fixture" }, ({path}) => ({ contents: {
      pdfjs: "export const GlobalWorkerOptions={};export const getDocument=()=>globalThis.__pdf.task;",
      worker: 'export default "fixture-worker";',
      native: "export const invoke=(...args)=>globalThis.__pdf.ocr(...args);",
      media: "export const analyzeVideoFile=()=>{},transcribeAudioFile=()=>{};",
    }[path], loader: "js" }));
  },
}] });
const { inferChatMode } = await import(pathToFileURL(output));
const image = { kind: "image", mediaItems: [{ id: "synthetic-frame" }] };
const pdf = { kind: "pdf", mediaItems: [] };
try {
  test("explicit media commands override incidental search/document topics in three languages", () => {
    const cases = [
      ["Generate an image for today's news", "image"],
      ["Generate a voiceover of the latest report", "voice"],
      ["Create a video introduction for a report", "video"],
      ["Please make a video about today's news", "video"],
      ["Could you generate a voice narration of the current report", "voice"],
      ["Generate background music for today's news report", "sound"],
      ["Create a voiceover for an image", "voice"],
      ["Create an image for a video", "image"],
      ["  GENERATE AN IMAGE for the latest report  ", "image"],
      ["আজকের খবরের জন্য একটি ছবি তৈরি করো", "image"],
      ["সর্বশেষ খবরের ভিডিও তৈরি করো", "video"],
      ["আজকের রিপোর্টের ভয়েসওভার তৈরি করো", "voice"],
      ["খবরের জন্য সঙ্গীত বানাও", "sound"],
      ["Создай изображение для последних новостей", "image"],
      ["Пожалуйста, создай видео для отчета сегодня", "video"],
      ["Сгенерируй озвучку последних новостей", "voice"],
      ["Создай музыку для новостей сегодня", "sound"],
    ];
    for (const [prompt, expected] of cases) {
      for (const attachments of [[], [image], [pdf], [image, pdf]]) {
        assert.equal(inferChatMode(prompt, attachments), expected, prompt);
      }
    }
  });
  test("analysis, research, search and document routes retain their fallbacks", () => {
    const cases = [
      ["Analyze this picture", [image], "vision"],
      ["Summarize this document", [pdf], "pdf"],
      ["Deep research into today's news", [], "research"],
      ["Search today's image generation news", [], "search"],
      ["Write a report about image generation", [], "document"],
      ["Create a report about image generation", [], "document"],
      ["Create a function to generate images for news", [], "search"],
      ["Explain how to generate an image for today's news", [], "search"],
      ["Make a document comparing video tools", [], "document"],
      ["What are today's video creation tools?", [], "search"],
      ["আজকের ছবি তৈরির খবর খুঁজে দাও", [], "search"],
      ["Найди последние новости о создании видео", [], "search"],
      ["hello", [], "chat"],
    ];
    for (const [prompt, attachments, expected] of cases) assert.equal(inferChatMode(prompt, attachments), expected, prompt);
  });
} finally { await rm(dir, { recursive: true, force: true }); }
