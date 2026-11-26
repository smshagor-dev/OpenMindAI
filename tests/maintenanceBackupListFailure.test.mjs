import { URL } from "node:url";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("../src/components/MaintenanceCenter.tsx", import.meta.url), "utf8");

test("initial backup listing handles rejection without an unhandled promise", () => {
  assert.match(source, /api\.listBackups\(\)\.then\([\s\S]*?\.catch\(\(error: unknown\)/);
  assert.match(source, /if \(mounted\) setBackupListError\(formatError\(error\)\)/);
  assert.match(source, /return \(\) => \{ mounted = false; \}/);
});

test("successful backup stays successful when post-create listing fails", () => {
  const start = source.indexOf("const createBackup = async");
  const end = source.indexOf("\n  return (", start);
  const handler = source.slice(start, end);
  assert.ok(handler.indexOf("setBackupSuccess(") < handler.indexOf("setBackups(await api.listBackups())"));
  assert.match(handler, /try \{\s*setBackups\(await api\.listBackups\(\)\);[\s\S]*?catch \(error\) \{\s*setBackupListError/);
  assert.match(source, /role="alert" className="setup-warning">Could not refresh database snapshots/);
});
