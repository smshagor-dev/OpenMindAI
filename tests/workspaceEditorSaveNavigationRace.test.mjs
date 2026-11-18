import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("../src/components/ProjectLocalWorkspace.tsx", import.meta.url), "utf8");

// Regression guards for asynchronous save acknowledgements: navigation must not
// replace the editor while an outstanding write can still update its revision.
test("file navigation refuses to replace editor during a pending operation", () => {
  assert.match(source, /const openEntry = async[\\s\\S]*?if \\(busy !== null \\|\\| !activeRoot/);
});

test("folder switching refuses to reset editor while a save is pending", () => {
  assert.match(source, /root\\.id === activeRoot\\?\\.id \\|\\| busy !== null/);
});

test("path navigation and detach cannot race an editor save", () => {
  for (const method of ["goToLocation", "goUp", "detachFolder"]) {
    const body = source.slice(source.indexOf(\`const \${method} = async\`));
    assert.ok(body.startsWith(\`const \${method} = async\`) && body.slice(0, 180).includes("busy !== null"), method);
  }
});
