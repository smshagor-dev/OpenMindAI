import test from "node:test";
import assert from "node:assert/strict";
import { parseDisplayedAttachmentSection } from "../src/lib/displayAttachmentSection.ts";

test("does not hide user-authored malformed attachment markers", () => {
  const prompt = "Please explain:\n\n[Attachment: this is literal text\nDo not remove this.";
  assert.deepEqual(parseDisplayedAttachmentSection(prompt), { start: -1, names: [] });
});

test("recognizes a valid serialized attachment section and its name", () => {
  const prompt = "Review this\n\n[Attachment: notes.txt, text, 120 B, text/plain]\nactual content";
  assert.deepEqual(parseDisplayedAttachmentSection(prompt), {
    start: "Review this".length,
    names: ["notes.txt"],
  });
});

test("skips malformed markers before a real attachment header", () => {
  const prompt = "Question\n\n[Attachment: example\n\n[Attachment: report.pdf, pdf, 1 KB, application/pdf]\nPage 1";
  const parsed = parseDisplayedAttachmentSection(prompt);
  assert.equal(prompt.slice(parsed.start + 2).startsWith("[Attachment: report.pdf"), true);
  assert.deepEqual(parsed.names, ["report.pdf"]);
});

test("supports attachment-only payload and preserves unrelated text", () => {
  assert.deepEqual(parseDisplayedAttachmentSection("[Attachment: photo.png, image, 2 KB, image/png]"), {
    start: 0,
    names: ["photo.png"],
  });
  assert.deepEqual(parseDisplayedAttachmentSection("Hello\n\n[Attachment: fake, text]"), {
    start: -1,
    names: [],
  });
});
