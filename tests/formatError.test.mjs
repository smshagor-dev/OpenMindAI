import test from "node:test";
import assert from "node:assert/strict";
import { formatError } from "../src/lib/format.ts";

test("preserves ordinary error and primitive representations", () => {
  assert.equal(formatError(new Error("disk unavailable")), "disk unavailable");
  assert.equal(formatError("failed"), "failed");
  assert.equal(formatError({ error: "backend unavailable" }), "backend unavailable");
  assert.equal(formatError({ reason: "cancelled" }), "cancelled");
  assert.equal(formatError({ code: 17 }), '{"code":17}');
  assert.equal(formatError(null), "null");
});

test("skips throwing metadata getters and reads the next safe message", () => {
  const failure = {
    get message() { throw new Error("getter exploded"); },
    error: "real failure",
  };
  assert.equal(formatError(failure), "real failure");
});

test("does not throw on hostile proxies, cycles, or coercion", () => {
  const prototypeTrap = new Proxy({}, {
    getPrototypeOf() { throw new Error("prototype blocked"); },
  });
  const fieldTrap = new Proxy({}, {
    get() { throw new Error("field blocked"); },
  });
  const cyclic = {};
  cyclic.self = cyclic;
  const hostilePrimitive = { [Symbol.toPrimitive]() { throw new Error("coercion blocked"); } };

  for (const failure of [prototypeTrap, fieldTrap, cyclic, hostilePrimitive]) {
    assert.doesNotThrow(() => formatError(failure));
    assert.equal(formatError(failure), "Unknown application error");
  }
});

test("falls back when Error.message accessor throws", () => {
  const error = new Error("original");
  Object.defineProperty(error, "message", {
    get() { throw new Error("message getter blocked"); },
    enumerable: true,
  });
  assert.equal(formatError(error), "Unknown application error");
});
