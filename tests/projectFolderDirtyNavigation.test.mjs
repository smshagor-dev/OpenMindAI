import { URL } from "node:url";
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/components/ProjectLocalWorkspace.tsx", import.meta.url), "utf8");

test("root tabs reject dirty navigation before invalidating directory selection", () => {
  const start = source.indexOf('"switch attached folders"');
  const selection = source.indexOf("directoryGate.current.select(props.projectId, root.id);", start);
  assert.ok(start > 0 && selection > start);
  assert.match(source.slice(start - 100, selection), /confirmWorkspaceDiscard\(editorDirty, "switch attached folders"\)/);
});

test("attaching and detaching folders require consent for unsaved editor changes", () => {
  assert.match(source, /if \(editorDirty && !window\.confirm\("Discard unsaved editor changes before attaching another folder\?"\)\) return;/);
  assert.match(source, /if \(editorDirty && !window\.confirm\("Discard unsaved editor changes before detaching this folder\?"\)\) return;/);
});
