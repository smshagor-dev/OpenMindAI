import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

// Both project surfaces must reject files above the persistence limit before
// writing a metadata-only database row or starting a binary attachment copy.
for (const path of [
  "src/components/ProjectsWorkspace.tsx",
  "src/components/WorkWorkspace.tsx",
]) {
  test(`${path}: oversized uploads are rejected before persistence`, () => {
    const source = readFileSync(path, "utf8");
    const handler = source.slice(source.indexOf("for (const file of Array.from(files))"));
    assert.ok(handler.startsWith("for (const file of Array.from(files))"));
    const reject = handler.indexOf("if (file.size > MAX_PROJECT_FILE_BYTES)");
    const attach = handler.indexOf("await attachProjectBinaryFile(activeProject.id, file)");
    const persist = handler.indexOf("await api.addProjectFile(");
    assert.ok(reject >= 0, "missing size guard");
    assert.ok(reject < attach, "must reject before binary attachment");
    assert.ok(reject < persist, "must reject before database insertion");
    assert.match(handler.slice(reject, attach), /throw new Error\(/);
    assert.match(handler.slice(reject, attach), /nothing was saved/);
  });
}
