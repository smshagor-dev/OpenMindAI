import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const settings = readFileSync(new URL('../src/components/SettingsDialog.tsx', import.meta.url), 'utf8');

test('capabilities panel does not offer a nonfunctional thinking toggle', () => {
  const capabilities = settings.split('activeSection === "capabilities"')[1]?.split('activeSection === ')[0];
  assert.ok(capabilities, 'capabilities section exists');
  assert.match(capabilities, /label="Thinking mode" value="Not configurable in this release"/);
  assert.doesNotMatch(capabilities, /setPreference\("thinkingModeEnabled"/);
  assert.doesNotMatch(capabilities, /checked=\{preferenceDraft\.thinkingModeEnabled\}/);
});

test('explains model-dependent reasoning without claiming request-level control', () => {
  assert.match(settings, /Model reasoning is controlled by the selected model and runtime/);
  assert.match(settings, /previous Thinking mode switch only saved a preference and did not\s+change inference behavior/);
});
