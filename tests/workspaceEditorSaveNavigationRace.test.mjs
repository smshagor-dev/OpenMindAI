import { URL } from "node:url";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("../src/components/ProjectLocalWorkspace.tsx", import.meta.url), "utf8");

function bodyFor(name) {
  const marker = "const " + name + " = async";
  const start = source.indexOf(marker);
  assert.ok(start >= 0, name + " handler exists");
  return source.slice(start, start + 350);
}

test("file navigation is blocked while a save is pending", () => {
  assert.ok(bodyFor("openEntry").includes("busy !== null"));
});

test("root tab changes are blocked while a save is pending", () => {
  assert.ok(source.includes("root.id === activeRoot?.id || busy !== null"));
});

test("path navigation and detach are blocked while a save is pending", () => {
  for (const name of ["goToLocation", "goUp", "detachFolder"]) {
    assert.ok(bodyFor(name).includes("busy !== null"), name);
  }
});
