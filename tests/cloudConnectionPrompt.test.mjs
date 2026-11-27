import test from "node:test";
import assert from "node:assert/strict";
import { cloudConnectionPromptReason } from "../src/lib/cloudConnectionPrompt.ts";

test("invalid Cloud credentials request the secure API-key dialog", () => {
  assert.match(cloudConnectionPromptReason(new Error("HTTP 401: Invalid API key.")), /valid API key/);
  assert.match(cloudConnectionPromptReason("Connect an OpenMindAI API key before using Cloud research"), /valid API key/);
});

test("Cloud transport failures offer connection recovery", () => {
  assert.match(cloudConnectionPromptReason(new Error("Cloud API is unreachable. Check your internet connection and retry.")), /could not be reached/);
});

test("ordinary Cloud capability and validation errors do not open the key dialog", () => {
  assert.equal(cloudConnectionPromptReason(new Error("No enabled hosted model is available")), null);
  assert.equal(cloudConnectionPromptReason(new Error("HTTP 422: invalid model")), null);
});
