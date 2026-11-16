import test, { before, beforeEach, after, afterEach } from "node:test";
import assert from "node:assert/strict";
import { act } from "react";
import { createChatAppHarness } from "./helpers/chatAppHarness.mjs";
let app;
before(async () => { app = await createChatAppHarness(); });
beforeEach(async () => { await app.mount(); });
afterEach(async () => { await app.unmount(); });
after(async () => { await app.dispose(); });
async function append(target, text) { let result; await act(async () => { result = target.append(text); }); return result; }

test("delayed transcript appends to the latest typed text, not the recording-start prompt", async () => {
  await app.type("Initial note");
  const target = app.props.createVoiceDraftTarget();
  await app.type("Initial note plus a new idea");
  assert(await append(target, " spoken idea "));
  assert.equal(app.props.prompt, "Initial note plus a new idea spoken idea");
});
test("clearing then typing again discards the old recording lease", async () => {
  await app.type("Initial note"); const target = app.props.createVoiceDraftTarget();
  await app.type(""); await app.type("New intentional draft");
  assert.equal(await append(target, "old speech"), false);
  assert.equal(app.props.prompt, "New intentional draft");
});
test("clearing an already empty draft also discards pending transcription", async () => {
  await app.type(""); const target = app.props.createVoiceDraftTarget();
  await app.type("");
  assert.equal(await append(target, "old speech"), false);
  assert.equal(app.props.prompt, "");
});
test("A to B to A navigation never restores A's discarded transcript", async () => {
  const target = app.props.createVoiceDraftTarget();
  await app.open("B"); await app.open("A"); await app.type("new A");
  assert.equal(await append(target, "old A speech"), false);
  assert.equal(app.props.prompt, "new A");
});
test("switching chats and New chat protect their own drafts", async () => {
  const target = app.props.createVoiceDraftTarget();
  await app.open("B"); await app.type("B draft");
  assert.equal(await append(target, "A speech"), false);
  assert.equal(app.props.prompt, "B draft");
  const next = app.props.createVoiceDraftTarget(); await app.newChat(); await app.type("new chat");
  assert.equal(await append(next, "B speech"), false);
  assert.equal(app.props.prompt, "new chat");
});
test("empty recognition results preserve text and a fresh recording can still append", async () => {
  await app.type("বাংলা"); const target = app.props.createVoiceDraftTarget();
  assert.equal(await append(target, "   "), false);
  assert.equal(app.props.prompt, "বাংলা");
  assert(await append(target, "কথা")); assert.equal(app.props.prompt, "বাংলা কথা");
});
