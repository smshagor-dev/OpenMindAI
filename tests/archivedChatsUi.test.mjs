import { URL } from "node:url";
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const sidebar = readFileSync(new URL("../src/components/Sidebar.tsx", import.meta.url), "utf8");
const archive = readFileSync(new URL("../src/components/ArchivedChats.tsx", import.meta.url), "utf8");

test("archived chat recovery is reachable from sidebar and invokes backend restore", () => {
  assert.match(sidebar, /onOpenArchived=\{?[^\n]*|onOpenArchived: \(\) => void/);
  assert.match(sidebar, /title="Archived chats"/);
  assert.match(app, /<ArchivedChats/);
  assert.match(app, /await api\.restoreConversation\(conversation\.id\)/);
});
test("restore failure keeps archived row for retry, success removes it", () => {
  assert.match(archive, /await onRestore\(conversation\)/);
  assert.match(archive, /setRows\(\(items\) => items\.filter\(\(item\) => item\.id !== conversation\.id\)\)/);
  assert.match(archive, /catch \(reason\)/);
  assert.match(archive, /setError\(String\(reason\)\)/);
});
test("async archive reads do not reopen a dismissed modal with stale data", () => {
  assert.match(archive, /const ticket = \+\+version\.current/);
  assert.match(archive, /ticket === version\.current/);
  assert.match(archive, /return \(\) => \{ version\.current \+= 1; \}/);
});
