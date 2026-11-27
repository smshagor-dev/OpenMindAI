import test from "node:test";
import assert from "node:assert/strict";
import {
  beginSetupPreparation,
  completeSetupPreparation,
  setupExecutionMode,
  setupPreparationPending,
} from "../src/lib/setupExecutionMode.ts";

const MODE = "openmindai.setup.executionMode.v1";
const PENDING = "openmindai.setup.preparing.v1";

function withStorage(run, failure = () => false) {
  const values = new Map();
  const storage = {
    getItem(key) { return values.has(key) ? values.get(key) : null; },
    setItem(key, value) {
      if (failure("set", key)) throw new Error("storage write denied");
      values.set(key, String(value));
    },
    removeItem(key) {
      if (failure("remove", key)) throw new Error("storage removal denied");
      values.delete(key);
    },
  };
  const previous = globalThis.window;
  globalThis.window = { localStorage: storage };
  try { run(values); } finally {
    if (previous === undefined) delete globalThis.window;
    else globalThis.window = previous;
  }
}

test("valid setup is published only after mode and pending marker are stored", () => {
  withStorage(values => {
    beginSetupPreparation("local");
    assert.equal(setupExecutionMode(), "local");
    assert.equal(setupPreparationPending(), true);
    completeSetupPreparation();
    assert.equal(setupPreparationPending(), false);
    assert.equal(values.get(MODE), "local");
  });
});

test("malformed runtime modes never overwrite an existing setup", () => {
  withStorage(values => {
    beginSetupPreparation("cloud");
    for (const invalid of ["false", "", null, undefined, {}, 1]) {
      assert.throws(() => beginSetupPreparation(invalid), TypeError);
      assert.equal(values.get(MODE), "cloud");
      assert.equal(values.get(PENDING), "pending");
    }
  });
});

test("failed mode write cannot reuse an old pending marker", () => {
  withStorage(values => {
    values.set(MODE, "cloud");
    values.set(PENDING, "pending");
    assert.throws(() => beginSetupPreparation("local"), /storage write denied/);
    assert.equal(setupPreparationPending(), false);
    assert.equal(values.get(MODE), "cloud");
  }, (operation, key) => operation === "set" && key === MODE);
});

test("failed pending write clears marker and never resumes partially written mode", () => {
  withStorage(values => {
    values.set(MODE, "cloud");
    values.set(PENDING, "pending");
    assert.throws(() => beginSetupPreparation("local"), /storage write denied/);
    assert.equal(values.get(MODE), "local");
    assert.equal(values.has(PENDING), false);
    assert.equal(setupPreparationPending(), false);
  }, (operation, key) => operation === "set" && key === PENDING);
});

test("failed initial marker removal prevents any mode mutation", () => {
  withStorage(values => {
    values.set(MODE, "cloud");
    assert.throws(() => beginSetupPreparation("local"), /storage removal denied/);
    assert.equal(values.get(MODE), "cloud");
  }, (operation, key) => operation === "remove" && key === PENDING);
});
