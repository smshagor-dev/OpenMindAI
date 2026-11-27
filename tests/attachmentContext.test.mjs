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


test("very long attachment headers cannot overflow the total prompt budget", () => {
  const sources = Array.from({ length: 8 }, (_, index) => ({
    name: "long-" + index,
    header: "[Attachment " + "x".repeat(100_000) + "]",
    content: "START" + "a".repeat(30_000) + "END",
  }));
  const { text, plan } = serializeAttachmentContext(sources, "query");
  assert(text.length <= 16_000);
  assert.equal(plan.items.length, 8);
  assert.equal(plan.truncatedCount, 8);
  assert(text.includes("omitted text was not analyzed"));
});

test("more than 64 attachments are disclosed rather than silently overflowing context", () => {
  const sources = Array.from({ length: 200 }, (_, index) => ({
    name: "file" + index,
    header: "[Attachment " + index + "]",
    content: "content-" + index,
  }));
  const { text, plan } = serializeAttachmentContext(sources, "");
  assert(text.length <= 16_000);
  assert.equal(plan.items.length, 200);
  assert.equal(plan.omittedCount, 136);
  assert(text.includes("[Attachment 63]"));
  assert(!text.includes("[Attachment 64]"));
  assert(text.includes("Additional attachments omitted from context"));
});

test("oversized prompts use bounded relevance terms without losing normal queries", () => {
  const content = "start " + "x".repeat(4000) + " boundedneedle answer " + "y".repeat(14000);
  const ordinary = selectAttachmentExcerpt(content, 4000, "find boundedneedle");
  assert(ordinary.includes("boundedneedle answer"));
  const oversized = selectAttachmentExcerpt(content, 4000, "unrelated ".repeat(500) + "boundedneedle");
  assert(!oversized.includes("boundedneedle answer"));
  assert(oversized.length <= 4000);
});

test("prompt prefix ending in a high surrogate is handled safely", () => {
  const content = "start " + "x".repeat(4000) + " unicodeprobe answer " + "y".repeat(14000);
  const prompt = "unicodeprobe " + " ".repeat(4082) + "\uD83D" + "\uDE00".repeat(10000);
  const excerpt = selectAttachmentExcerpt(content, 4000, prompt);
  assert(excerpt.includes("unicodeprobe answer"));
  assert(excerpt.length <= 4000);
});

const hasUnpairedSurrogate = (text) =>
  /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u.test(text);

test("truncated head, middle and tail windows preserve Unicode scalar boundaries", () => {
  // Dense supplementary characters exercise both odd and even UTF-16 offsets.
  const content = "😀".repeat(15000);
  for (const limit of [70, 80, 101, 500, 999, 2048, 4095, 8000]) {
    const excerpt = selectAttachmentExcerpt(content, limit, "unmatched search");
    assert(excerpt.length <= limit, "budget exceeded at " + limit);
    assert(!hasUnpairedSurrogate(excerpt), "surrogate split at " + limit);
    assert(excerpt.includes("Attachment excerpt truncated"));
  }
});

test("relevance-aligned middle window and small-limit head remain valid UTF-16", () => {
  const content = "😀".repeat(4000) + " targetterm " + "🚀".repeat(4000);
  for (const limit of [75, 99, 600, 1001, 3000, 6001]) {
    const excerpt = selectAttachmentExcerpt(content, limit, "targetterm");
    assert(excerpt.length <= limit);
    assert(!hasUnpairedSurrogate(excerpt), "split at " + limit);
  }
  assert(selectAttachmentExcerpt(content, 6001, "targetterm").includes("targetterm"));
});

test("NFKC-expanded prefix does not displace the query-relevant middle excerpt", () => {
  const content = "ﬃ".repeat(4000) + " targetdiscovery 42 " + "z".repeat(12000);
  const excerpt = selectAttachmentExcerpt(content, 3000, "Find targetdiscovery");
  assert(excerpt.includes("targetdiscovery 42"));
  assert(excerpt.length <= 3000);
});
