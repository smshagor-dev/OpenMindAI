import assert from "node:assert/strict";
import test from "node:test";
import { selectAttachmentExcerpt, serializeAttachmentContext } from "../src/lib/attachmentContext.ts";

test("tiny nonzero budgets preserve source evidence rather than partial warning", () => {
  const source = "Evidence: signed receipt is present. ".repeat(20);
  for (const limit of [1, 8, 32, 70]) {
    const excerpt = selectAttachmentExcerpt(source, limit, "receipt");
    assert.equal(excerpt, source.slice(0, limit));
    assert.equal(excerpt.length, limit);
  }
});

test("zero and untruncated budgets keep existing behavior", () => {
  assert.equal(selectAttachmentExcerpt("abcdef", 0, ""), "");
  assert.equal(selectAttachmentExcerpt("abcdef", 6, ""), "abcdef");
  assert.equal(selectAttachmentExcerpt("abcdef", 100, ""), "abcdef");
});

test("larger excerpts retain an explicit truncation warning and respect the cap", () => {
  const source = "alpha ".repeat(200);
  const excerpt = selectAttachmentExcerpt(source, 120, "alpha");
  assert.ok(excerpt.startsWith("alpha"));
  assert.match(excerpt, /Attachment excerpt truncated/);
  assert.ok(excerpt.length <= 120);
});

test("serialized context still discloses omitted evidence", () => {
  const sources = [{ name: "evidence.txt", header: "File: evidence.txt", content: "proof ".repeat(2000) }];
  const { text, plan } = serializeAttachmentContext(sources, "proof");
  assert.equal(plan.truncatedCount, 1);
  assert.match(text, /Attachment context limit/);
  assert.match(text, /proof/);
});


test("tiny Unicode budgets never split surrogate pairs", () => {
  assert.equal(selectAttachmentExcerpt("😀".repeat(100), 1, ""), "");
  assert.equal(selectAttachmentExcerpt("😀".repeat(100), 3, ""), "😀");
});
