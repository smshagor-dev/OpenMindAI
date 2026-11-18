import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("../src/components/ProjectLocalWorkspace.tsx", import.meta.url), "utf8");
const deleteHandler = source.split("const deleteItem = async () => {")[1]?.split("const goToLocation = async () => {")[0];

test("deleting an edited file or ancestor requires dirty-editor confirmation", () => {
  assert.ok(deleteHandler, "delete handler exists");
  assert.match(deleteHandler, /editorPath === deleteTarget\.relativePath/);
  assert.match(deleteHandler, /deleteTarget\.kind === "directory"/);
  assert.match(deleteHandler, /editorPath\.startsWith/);
  assert.match(deleteHandler, /editedFileIsTarget && !confirmWorkspaceDiscard\(editorDirty/);
  assert.ok(deleteHandler.indexOf("confirmWorkspaceDiscard(editorDirty") < deleteHandler.indexOf('run("delete-path"'),
    "confirmation must precede filesystem mutation");
});

test("unrelated deletion is not blocked by an unsaved editor buffer", () => {
  assert.match(deleteHandler, /if \(editedFileIsTarget && !confirmWorkspaceDiscard/);
  assert.doesNotMatch(deleteHandler, /if \(editorDirty && !confirmWorkspaceDiscard/);
});
