import { URL } from "node:url";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

// Exercise the production scale expression without loading pdfjs/Tauri in Node.
const source = readFileSync(new URL("../src/lib/pdf.ts", import.meta.url), "utf8");
const expression = source.match(/const scale = (Math\.min\(2, MAX_RENDER_DIMENSION \/ Math\.max\(base\.width, base\.height\)\));/);
assert.ok(expression, "production render scale must enforce the pixel cap");
const scaleFor = new Function("base", "MAX_RENDER_DIMENSION", `return ${expression[1]};`);
const limit = 1440;

test("huge page never exceeds the 1440-pixel render dimension", () => {
  for (const [width, height] of [[20_000, 1_000], [1_000, 20_000], [1_000_000, 1_000_000]]) {
    const scale = scaleFor({ width, height }, limit);
    assert.ok(Math.max(width, height) * scale <= limit);
    assert.ok(scale < 0.5, "oversized pages require a scale below the old floor");
  }
});

test("ordinary pages retain high-resolution rendering", () => {
  assert.equal(scaleFor({ width: 600, height: 400 }, limit), 2);
  assert.equal(scaleFor({ width: 1440, height: 720 }, limit), 1);
});

test("invalid PDF media boxes are rejected before canvas allocation", () => {
  assert.match(source, /!Number\.isFinite\(base\.width\)/);
  assert.match(source, /!Number\.isFinite\(base\.height\)/);
  assert.match(source, /base\.width <= 0 \|\| base\.height <= 0/);
  assert.doesNotMatch(source, /Math\.max\(0\.5, scale\)/);
});
