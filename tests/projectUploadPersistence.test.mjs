import { URL } from "node:url";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

// Source-level guard for the Tauri/React integration boundary. The desktop
// attachment command requires a running Tauri host and is exercised separately.
const source = readFileSync(new URL("../src/components/ProjectsWorkspace.tsx", import.meta.url), "utf8");
const upload = source.slice(source.indexOf("const addFiles = async"), source.indexOf("const saveBinaryCopy = async"));

test("oversized uploads fail before any metadata or binary write", () => {
  const sizeCheck = upload.indexOf("file.size > MAX_PROJECT_FILE_BYTES");
  const binaryWrite = upload.indexOf("await attachProjectBinaryFile(");
  const metadataWrite = upload.indexOf("await api.addProjectFile(");
  assert.ok(sizeCheck >= 0);
  assert.ok(sizeCheck < binaryWrite && sizeCheck < metadataWrite);
  assert.match(upload, /File exceeds the 1\.5 MB project attachment limit; nothing was saved/);
});

test("unsupported non-text files go to the durable binary attachment path", () => {
  assert.match(upload, /if \(!isTextFile\(file\)\)\s*\{\s*await attachProjectBinaryFile\(activeProject\.id, file\)/);
});

test("failed text extraction cannot create a misleading project file record", () => {
  const failureGuard = upload.indexOf('ingestion.status !== "ready"');
  const metadataWrite = upload.indexOf("await api.addProjectFile(");
  assert.ok(failureGuard >= 0 && failureGuard < metadataWrite);
  assert.match(upload, /File could not be read; nothing was saved/);
});

test("failed files are surfaced without preventing other files from uploading", () => {
  assert.match(upload, /catch \(caught\) \{\s*failures\.push/);
  assert.match(upload, /await refreshProjects\(activeProject\.id\)/);
});
