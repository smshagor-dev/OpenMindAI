import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (file) => readFileSync(new globalThis.URL(`../${file}`, import.meta.url), "utf8");
const css = read("src/styles.css");
const models = read("src/components/ModelsManager.tsx");
const datasets = read("src/components/DatasetsManager.tsx");
const wizard = read("src/components/SetupWizardDesktop.tsx");

function block(selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = css.match(new RegExp(`${escaped}\\s*\\{([^{}]*)\\}`));
  assert.ok(match, `Missing CSS rule: ${selector}`);
  return match[1];
}

test("startup chat surface is derived from selected theme, not dark-only colors", () => {
  const background = block(".messages-empty");
  assert.match(background, /var\(--bg\)/);
  assert.match(background, /var\(--text\)/);
  assert.doesNotMatch(background, /#050505|rgba\(255,\s*255,\s*255/);
  assert.match(block(".suggestion-row-item:hover"), /var\(--accent\)/);
  assert.match(css, /:root\[data-theme="light"\] \.chat-empty-inner \.composer-box/);
});

test("model download status is visible with accessible progress and failure messaging", () => {
  assert.match(models, /role="progressbar"/);
  assert.match(models, /aria-valuenow=/);
  assert.match(models, /download progress/);
  assert.match(models, /className="download-feedback-error" role="alert"/);
  assert.match(models, /anotherDownloadActive/);
  assert.match(css, /\.download-progress-track,\s*\.dataset-progress-track,\s*\.setup-progress-bar\s*\{[^}]*var\(--border-strong\)/);
  assert.match(css, /\.model-download-card \.download-progress > span\s*\{[^}]*var\(--text\)/);
});

test("dataset downloads show theme-aware status, errors and persist later completion", () => {
  assert.match(datasets, /className="dataset-download-status" role="status" aria-live="polite"/);
  assert.match(datasets, /role="progressbar"/);
  assert.match(datasets, /downloadStatus\.error \? <p className="download-feedback-error"/);
  assert.match(datasets, /downloadStatus\?\.state !== "completed"/);
  assert.match(datasets, /window\.localStorage\.setItem\(/);
  assert.match(datasets, /anotherDownloadActive/);
  assert.match(block(".dataset-download-status .model-badge.downloaded"), /var\(--success\)/);
});

test("setup wizard uses the same error and progress styles as model manager", () => {
  assert.match(wizard, /className="setup-model-catalog"/);
  assert.match(wizard, /className="setup-progress-bar"/);
  assert.match(css, /\.setup-model-catalog \.model-selector-error/);
  assert.match(css, /:root\[data-theme="light"\] \.setup-wizard/);
  assert.match(block(".download-feedback-error"), /var\(--danger-bg\)/);
});
