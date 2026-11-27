import test from "node:test";
import assert from "node:assert/strict";
import { formatTime } from "../src/lib/format.ts";

test("valid ISO timestamps produce a time and invalid strings produce no label", () => {
  assert.match(formatTime("2026-11-21T08:30:00Z"), /\d/);
  assert.equal(formatTime("not-a-date"), "");
  assert.equal(formatTime(""), "");
  assert.equal(formatTime("  "), "");
});

test("untrusted non-string timestamps never crash the display formatter", () => {
  const throwingCoercion = {
    [Symbol.toPrimitive]() { throw new Error("malicious timestamp"); },
  };
  for (const value of [null, undefined, 0, NaN, Symbol("timestamp"), throwingCoercion]) {
    assert.doesNotThrow(() => formatTime(value));
    assert.equal(formatTime(value), "");
  }
});

test("valid timestamp formatting remains deterministic for repeated calls", () => {
  const iso = "2026-11-21T08:30:00+00:00";
  assert.equal(formatTime(iso), formatTime(iso));
  assert.notEqual(formatTime(iso), "");
});
