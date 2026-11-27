import { URL } from "node:url";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../src/lib/documentIntent.ts", import.meta.url), "utf8");
const executable = source
  .replace("export function requestedDocumentArtifact(prompt: string): \"pdf\" | \"docx\" | null", "function requestedDocumentArtifact(prompt)")
  + "\nrequestedDocumentArtifact";
const classify = vm.runInNewContext(executable);

test("explicit PDF and Word output requests produce the right artifact", () => {
  for (const [prompt, expected] of [
    ["Create a four-page project report as a PDF", "pdf"],
    ["Please generate a PDF report", "pdf"],
    ["Could you please create a Word document about robotics?", "docx"],
    ["Export this as a DOCX", "docx"],
  ]) assert.equal(classify(prompt), expected, prompt);
});

test("reading attachments and topic mentions never create an output file", () => {
  for (const prompt of [
    "Read this attached PDF",
    "Summarize the PDF document",
    "What is a PDF?",
    "Analyze this Word document",
    "Write an explanation of PDF compression",
    "Create a summary of the attached PDF",
  ]) assert.equal(classify(prompt), null, prompt);
});

test("chat completion persists the document with its assistant message", () => {
  const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
  assert.match(app, /requestedDocumentArtifact\(content\)/);
  assert.match(app, /api\.createDocumentArtifact\(\s*conversationId,\s*assistant\.id,\s*documentKind,/);
  assert.match(app, /requestScope\.setArtifacts\(\(items\) => upsertArtifactInList\(items, artifact\)\)/);
});
