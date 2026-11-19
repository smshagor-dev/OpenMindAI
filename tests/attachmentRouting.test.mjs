import test, { after } from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const directory = await mkdtemp(join(tmpdir(), "attachment-routing-"));
try {
  const output = join(directory, "chat.mjs");
  await build({
    entryPoints: ["src/lib/chat.ts"],
    outfile: output,
    bundle: true,
    format: "esm",
    platform: "node",
    plugins: [{
      name: "media-fixtures",
      setup(builder) {
        builder.onResolve({ filter: /^\.\/media$/ }, () => ({ path: "media", namespace: "fixture" }));
        builder.onResolve({ filter: /^\.\/pdf$/ }, () => ({ path: "pdf", namespace: "fixture" }));
        builder.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => ({
          contents: path === "pdf"
            ? "export async function extractPdfText(){ globalThis.__pdfReads++; return {text:'PDF BODY',pageCount:1,pagesRead:1,scannedPages:[],unreadPages:[],visionPages:[],warnings:[],truncated:false}; }"
            : "export async function transcribeAudioFile(){ globalThis.__audioReads++; return {durationSeconds:1,text:'AUDIO BODY'}; } export async function analyzeVideoFile(){throw Error('unexpected video')}",
        })),
      },
    }],
  });
  const { readAttachment } = await import(pathToFileURL(output).href);
  const file = (name, type) => ({
    name, type, size: 12,
    text: async () => { throw Error("binary media must not be read as UTF-8"); },
    arrayBuffer: async () => new ArrayBuffer(12),
  });
  test("PDF with text-looking suffix is processed as PDF", async () => {
    globalThis.__pdfReads = 0;
    const result = await readAttachment(file("scan.txt", "application/pdf"));
    assert.equal(result.kind, "pdf");
    assert.match(result.contentPreview, /PDF BODY/);
    assert.equal(globalThis.__pdfReads, 1);
  });
  test("audio with text-looking suffix is transcribed, not decoded as text", async () => {
    globalThis.__audioReads = 0;
    const result = await readAttachment(file("recording.txt", "audio/wav"));
    assert.equal(result.kind, "audio");
    assert.match(result.contentPreview, /AUDIO BODY/);
    assert.equal(globalThis.__audioReads, 1);
  });
  test("ordinary text files still use text reader and retain their content", async () => {
    const result = await readAttachment({ ...file("notes.txt", "text/plain"), text: async () => "hello বাংলা" });
    assert.equal(result.kind, "text");
    assert.equal(result.contentPreview, "hello বাংলা");
  });
} finally {
  after(async () => { await rm(directory, { recursive: true, force: true }); });
}
