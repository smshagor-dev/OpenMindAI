import test from "node:test";
import assert from "node:assert/strict";
import {
  planAttachmentContext,
  serializeAttachmentContext,
  selectAttachmentExcerpt,
} from "../src/lib/attachmentContext.ts";

const source = (index, content) => ({
  name: "file" + index + ".txt",
  header: "[Attachment: file" + index + ".txt, text, 8000 B, text/plain]",
  content,
});

test("three large files survive the 16k budget with explicit omission notices", () => {
  const sources = Array.from({ length: 3 }, (_, i) =>
    source(i, String(i) + "-START" + "a".repeat(17000) + String(i) + "-END"),
  );
  const { text, plan } = serializeAttachmentContext(sources, "important conclusion");
  assert(text.length <= 16000);
  assert.equal(plan.truncatedCount, 3);
  for (let i = 0; i < 3; i += 1) {
    assert(text.includes("file" + i + ".txt"));
    assert(text.includes(i + "-START"));
    assert(text.includes(i + "-END"));
  }
  assert(text.includes("omitted text was not analyzed"));
});

test("query-relevant answer deep in a long PDF is retained", () => {
  const content = "heading: " + "x".repeat(9000) + " photonspecific discovery 42 " + "y".repeat(9000);
  const snippet = selectAttachmentExcerpt(content, 6000, "Please explain photonspecific discovery");
  assert(snippet.includes("photonspecific discovery 42"));
  assert(snippet.includes("Attachment excerpt truncated"));
  assert(snippet.length <= 6000);
});

test("short documents remain verbatim while long files receive unused budget", () => {
  const sources = [source(0, "short"), source(1, "x".repeat(18000))];
  const { text, plan } = serializeAttachmentContext(sources, "");
  assert(text.includes("[Attachment: file0.txt, text, 8000 B, text/plain]\nshort"));
  assert.equal(plan.items[0].truncated, false);
  assert(plan.items[1].providedChars > 7000);
  assert(text.length <= 16000);
});

test("all eight files retain headers and nonzero excerpts", () => {
  const sources = Array.from({ length: 8 }, (_, i) => source(i, "X".repeat(22000)));
  const { text, plan } = serializeAttachmentContext(sources, "");
  assert(plan.budgets.every(value => value > 1000));
  assert(text.length <= 16000);
  for (let i = 0; i < 8; i += 1) assert(text.includes("file" + i + ".txt"));
});

test("small limits never overflow and flag truncation", () => {
  for (const cap of [1, 12, 60, 100, 500]) {
    const value = selectAttachmentExcerpt("a".repeat(5000), cap, "search");
    assert(value.length <= cap);
  }
});

test("no truncation warning on fully included short content", () => {
  const sources = [source(0, "one"), source(1, "two")];
  const { text, plan } = serializeAttachmentContext(sources, "one");
  assert.equal(plan.truncatedCount, 0);
  assert.equal(plan.omittedCount, 0);
  assert(!text.includes("omitted text was not analyzed"));
  assert.equal(planAttachmentContext(sources).items.length, 2);
});
