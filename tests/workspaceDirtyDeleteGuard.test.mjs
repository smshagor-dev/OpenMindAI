import { URL } from "node:url";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("../src/components/ProjectLocalWorkspace.tsx", import.meta.url), "utf8");

test("deleting an edited file requires discard confirmation before filesystem mutation", () => {
  const start = source.indexOf("const deleteItem = async () => {");
  const end = source.indexOf("const goToLocation = async () => {", start);
  assert.ok(start >= 0 && end > start);
  const handler = source.slice(start, end);
  assert.match(handler, /editorDirty && editorPath && workspacePathContains\(deleteTarget\.relativePath, editorPath\)/);
  assert.match(handler, /!confirmWorkspaceDiscard\(true, "delete the file containing unsaved editor changes"\)\) return;/);
  assert.ok(handler.indexOf("confirmWorkspaceDiscard") < handler.indexOf("localWorkspaceApi.deletePath("));
});

test("deletion containment checks exact paths and directory boundaries, including Windows separators", () => {
  const helper = source.slice(source.indexOf("function workspacePathContains("));
  assert.match(helper, /normalizedChild === normalizedParent/);
  assert.match(helper, /normalizedChild\.startsWith\(\x60\$\{normalizedParent\}\/\x60\)/);
  assert.match(helper, /replace\(\/\\\\\/g, "\/"\)/);
});
