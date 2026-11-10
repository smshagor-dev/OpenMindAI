import test from "node:test";
import assert from "node:assert/strict";
import { WorkspaceDirectoryGate } from "../src/lib/workspaceDirectoryGate.ts";

const deferred = () => {
  let resolve;
  const promise = new Promise((res) => { resolve = res; });
  return { promise, resolve };
};

async function load(gate, projectId, rootId, path, request) {
  const token = gate.begin(projectId, rootId, path);
  if (!token) return null;
  const entries = await request;
  return gate.accept(token) ? entries : null;
}

test("old root A response cannot replace a newer root B listing", async () => {
  const gate = new WorkspaceDirectoryGate();
  gate.select("project", "A");
  const slowA = deferred();
  const old = load(gate, "project", "A", "", slowA.promise);
  gate.select("project", "B");
  assert.equal(gate.isDisplayed("project", "A", ""), false);
  assert.deepEqual(await load(gate, "project", "B", "", Promise.resolve(["b.txt"])), ["b.txt"]);
  slowA.resolve(["a.txt"]);
  assert.equal(await old, null);
  assert.equal(gate.isDisplayed("project", "B", ""), true);
  assert.equal(gate.isDisplayed("project", "A", ""), false);
});

test("navigation invalidates stale entries before the next response", async () => {
  const gate = new WorkspaceDirectoryGate();
  gate.select("project", "A");
  await load(gate, "project", "A", "", Promise.resolve(["root.txt"]));
  const pending = deferred();
  const next = load(gate, "project", "A", "folder", pending.promise);
  assert.equal(gate.isDisplayed("project", "A", ""), false);
  assert.equal(gate.isDisplayed("project", "A", "folder"), false);
  pending.resolve(["nested.txt"]);
  assert.deepEqual(await next, ["nested.txt"]);
  assert.equal(gate.isDisplayed("project", "A", "folder"), true);
});

test("later refresh wins over a delayed earlier same-directory request", async () => {
  const gate = new WorkspaceDirectoryGate();
  gate.select("project", "A");
  const slow = deferred();
  const old = load(gate, "project", "A", "", slow.promise);
  assert.deepEqual(await load(gate, "project", "A", "", Promise.resolve(["new.txt"])), ["new.txt"]);
  slow.resolve(["old.txt"]);
  assert.equal(await old, null);
  assert.equal(gate.isDisplayed("project", "A", ""), true);
});

test("A to B to A selection rejects the initial A request", async () => {
  const gate = new WorkspaceDirectoryGate();
  gate.select("project", "A");
  const slow = deferred();
  const old = load(gate, "project", "A", "", slow.promise);
  gate.select("project", "B");
  gate.select("project", "A");
  await load(gate, "project", "A", "", Promise.resolve(["fresh"]));
  slow.resolve(["stale"]);
  assert.equal(await old, null);
  assert.equal(gate.isDisplayed("project", "A", ""), true);
});

test("project switches and stale root operations are rejected", async () => {
  const gate = new WorkspaceDirectoryGate();
  gate.select("first", "shared-id");
  await load(gate, "first", "shared-id", "", Promise.resolve(["first.txt"]));
  gate.select("second", "shared-id");
  assert.equal(gate.isDisplayed("first", "shared-id", ""), false);
  assert.equal(gate.begin("first", "shared-id", ""), null);
  assert.deepEqual(await load(gate, "second", "shared-id", "", Promise.resolve(["second.txt"])), ["second.txt"]);
});
