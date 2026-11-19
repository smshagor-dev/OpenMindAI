import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("../src/components/ProjectLocalWorkspace.tsx", import.meta.url), "utf8");
const deleteHandler = source.split("const deleteItem = async () => {")[1]?.split('run("delete-path"')[0];

test("deleting an edited file or ancestor confirms before removing it", () => {
  assert.ok(deleteHandler, "delete handler exists");
  assert.match(deleteHandler, /editorDirty && editorPath && workspacePathContains\(deleteTarget\.relativePath, editorPath\)/);
  assert.match(deleteHandler, /!confirmWorkspaceDiscard\(true,/);
  assert.ok(deleteHandler.indexOf("confirmWorkspaceDiscard(") >= 0, "confirmation occurs before delete");
});

test("unrelated deletion does not require discarding unsaved editor buffer", () => {
  assert.match(deleteHandler, /editorDirty && editorPath && workspacePathContains\(/);
  assert.doesNotMatch(deleteHandler, /if \(editorDirty && !confirmWorkspaceDiscard/);
});
