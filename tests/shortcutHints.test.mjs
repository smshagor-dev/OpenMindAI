import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("../src/components/SettingsDialog.tsx", import.meta.url), "utf8");

test("keyboard shortcut reference follows the persisted showShortcutHints preference", () => {
  assert.match(source, /activeSection === "shortcuts"[\s\S]*?preferenceDraft\?\.showShortcutHints/);
  assert.match(source, /preferenceDraft\?\.showShortcutHints[\s\S]*?<Info label="New chat"/);
  assert.match(source, /Optional shortcut hints are hidden/);
  assert.match(source, /Keyboard shortcuts still work/);
});

test("shortcut preference remains editable and saveable in Appearance", () => {
  assert.match(source, /label="Show shortcut hints"[\s\S]*?checked=\{preferenceDraft\.showShortcutHints\}/);
  assert.match(source, /onChange=\{\(value\) => setPreference\("showShortcutHints", value\)\}/);
});
