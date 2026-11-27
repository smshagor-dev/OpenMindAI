import { URL } from "node:url";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const listener = app.split('listen<StreamStartedEvent>("inference:started", (event) => {')[1]?.split('listen<{ conversationId: string; modelName: string; replacing: string | null }>')[0];

test("rejects inconsistent stream owners before mutating conversation state", () => {
  assert.ok(listener, "inference:started listener must exist");
  const guard = listener.indexOf("event.payload.assistant.conversationId !== event.payload.conversationId");
  const userGuard = listener.indexOf("event.payload.user.conversationId !== event.payload.conversationId");
  const start = listener.indexOf("chatState.startStream(event.payload)");
  assert.ok(guard >= 0 && userGuard >= 0, "both persisted message owners must be validated");
  assert.ok(start > guard && start > userGuard, "ownership validation must precede stream creation");
});

test("rejects missing conversation IDs before stream creation", () => {
  assert.ok(listener?.includes("!event.payload.conversationId"));
  assert.ok(listener.indexOf("!event.payload.conversationId") < listener.indexOf("chatState.startStream(event.payload)"));
});
