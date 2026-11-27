import test from "node:test";
import assert from "node:assert/strict";
import { formatError } from "../src/lib/format.ts";

test("formatError strips bidi overrides and isolate controls from untrusted errors", () => {
  assert.equal(formatError("failed \u202Etxt.exe\u202C \u2066server\u2069"), "failed txt.exe server");
  assert.equal(formatError(new Error("unsafe\u001b[31m\u0007")), "unsafe[31m");
  assert.equal(formatError({ message: "bad\u202Ddirection\u202C" }), "baddirection");
});

test("formatError preserves ordinary Unicode, newlines, tabs, and JSON fallback", () => {
  assert.equal(formatError("বাংলা 😀\nline\t2"), "বাংলা 😀\nline\t2");
  assert.equal(formatError({ code: "E_FAIL", details: "\u2067spoof\u2069" }), '{"code":"E_FAIL","details":"spoof"}');
  assert.equal(formatError(new Error("normal failure")), "normal failure");
});
