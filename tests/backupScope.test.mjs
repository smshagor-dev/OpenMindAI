import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DATABASE_BACKUP_NOTICE,
  DATABASE_BACKUP_RESTORE_GUIDE,
  backupScopeLabel,
} from "../src/lib/backupScope.ts";

test("database snapshot notice lists out-of-database evidence explicitly", () => {
  for (const required of [
    "Database-only",
    "NOT included",
    "uploaded media",
    "generated files",
    "datasets",
    "installed models",
  ]) {
    assert.ok(DATABASE_BACKUP_NOTICE.includes(required), required);
  }
});

test("restore guidance warns against treating a SQLite snapshot as complete", () => {
  for (const required of [
    "cannot restore a complete workspace",
    "close OpenMindAI",
    "entire portable storage root",
    "data/media",
    "data/media-index",
    "generated",
    "datasets",
    "active database",
  ]) {
    assert.ok(DATABASE_BACKUP_RESTORE_GUIDE.includes(required), required);
  }
});

test("scope labels fail closed for missing or future kinds", () => {
  assert.equal(backupScopeLabel("databaseOnly"), "Database only");
  for (const value of [undefined, null, "", "complete", "allFiles"]) {
    assert.equal(backupScopeLabel(value), "Backup scope unverified");
  }
});
