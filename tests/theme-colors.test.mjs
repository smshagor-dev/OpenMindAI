import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path) => readFileSync(new globalThis.URL(`../${path}`, import.meta.url), "utf8");
const stylesheet = read("src/styles.css");
const codeStyles = read("src/coding-workspace.css");
const startupStyles = read("src/startup.css");
const terminalStyles = read("src/projects-local-workspace.css");
const mediaStyles = read("src/multimodal.css");

function themeTokens(selector) {
  const start = stylesheet.indexOf(`${selector} {`);
  assert.notEqual(start, -1, `Theme selector missing: ${selector}`);
  const body = stylesheet.slice(start + selector.length + 2, stylesheet.indexOf("}", start));
  const tokens = new Map();
  for (const match of body.matchAll(/(--[a-z-]+):\s*([^;]+);/gi)) {
    tokens.set(match[1], match[2]);
  }
  return tokens;
}

function luminance(color) {
  const channels = color.match(/[0-9a-f]{2}/gi).map((part) => {
    const value = parseInt(part, 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
}

function contrast(foreground, background) {
  const a = luminance(foreground);
  const b = luminance(background);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

const foregroundPairs = [
  ["--text", "--bg"],
  ["--text", "--surface"],
  ["--text", "--sidebar"],
  ["--muted", "--bg"],
  ["--muted", "--surface"],
  ["--accent", "--bg"],
  ["--accent", "--surface"],
  ["--success", "--surface"],
  ["--warning", "--surface"],
  ["--status-error", "--surface"],
  ["--syntax-keyword", "--code-bg"],
  ["--syntax-string", "--code-bg"],
  ["--syntax-number", "--code-bg"],
  ["--syntax-comment", "--code-bg"],
  ["--syntax-title", "--code-bg"],
  ["--syntax-variable", "--code-bg"],
  ["--terminal-text", "--terminal-bg"],
  ["--terminal-muted", "--terminal-bg"],
  ["--terminal-success", "--terminal-bg"],
  ["--terminal-error", "--terminal-bg"],
];

test("light palette maintains WCAG AA text contrast across app surfaces", () => {
  const tokens = themeTokens(':root[data-theme="light"]');
  for (const [foreground, background] of foregroundPairs) {
    assert.ok(tokens.has(foreground), `Missing light token ${foreground}`);
    assert.ok(tokens.has(background), `Missing light token ${background}`);
    const ratio = contrast(tokens.get(foreground), tokens.get(background));
    assert.ok(ratio >= 4.5, `${foreground} on ${background} only has ${ratio.toFixed(2)}:1 contrast`);
  }
});

test("light and dark palettes define the same semantic and terminal colors", () => {
  const dark = themeTokens(":root");
  const light = themeTokens(':root[data-theme="light"]');
  for (const key of light.keys()) {
    assert.ok(dark.has(key), `Dark palette missing ${key}`);
  }
  for (const key of ["--accent", "--success", "--syntax-keyword", "--terminal-bg"]) {
    assert.notEqual(light.get(key), dark.get(key), `${key} must be theme-specific`);
  }
});

test("secondary UI surfaces use selected app palette instead of dark-only fallbacks", () => {
  assert.doesNotMatch(codeStyles, /--text-primary|--text-secondary|--surface-3|--border-subtle/);
  assert.match(codeStyles, /color: var\(--text\)/);
  assert.match(startupStyles, /background: var\(--surface-2\)/);
  assert.match(terminalStyles, /background: var\(--terminal-bg\)/);
  assert.match(terminalStyles, /color: var\(--terminal-text\)/);
  assert.doesNotMatch(mediaStyles, /prefers-color-scheme/);
  assert.match(mediaStyles, /color: var\(--status-error\)/);
});

test("appearance choice applies during boot and stays in sync with app settings", () => {
  const index = read("index.html");
  const app = read("src/App.tsx");
  const boot = read("public/theme-init.js");
  assert.match(index, /src="\/theme-init\.js"/);
  assert.match(boot, /document\.documentElement\.dataset\.theme = theme/);
  assert.match(boot, /openmindai:theme/);
  assert.match(app, /localStorage\.setItem\("openmindai:theme", preferences\.theme\)/);
  assert.match(app, /onSystemThemeChange/);
});
