import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { createChatAppHarness, message } from "./helpers/chatAppHarness.mjs";

test("App keeps the original reply visible while regeneration is pending, fails or is cancelled", async () => {
  const app = await createChatAppHarness();
  try {
    for (const outcome of ["failed", "cancelled", "completed"]) {
      await app.mount();
      const original = message("A", "original", "valuable original reply", "assistant");
      app.fixture.messages.A.push(original);
      await app.open("B");
      await app.open("A");
      let pending;
      await React.act(async () => { pending = app.props.regenerate(original.id); });
      assert(app.props.messages.some(item => item.id === original.id));
      if (outcome === "failed") {
        await app.fail({ pending, gate: app.fixture.regeneration });
        assert(app.props.messages.some(item => item.id === original.id));
      } else {
        const candidate = { ...message("A", "candidate", "replacement", "assistant"), status: outcome };
        // Native completion atomically retires the source only on success.
        app.fixture.messages.A = outcome === "completed"
          ? [app.fixture.messages.A[0], candidate]
          : [...app.fixture.messages.A, candidate];
        await React.act(async () => {
          app.fixture.regeneration.resolve(candidate);
          await pending;
        });
        assert.equal(app.props.messages.some(item => item.id === original.id), outcome !== "completed");
        assert(app.props.messages.some(item => item.id === "candidate"));
      }
      await app.unmount();
    }
  } finally { await app.dispose(); }
});
