import test, { before, beforeEach, after, afterEach } from "node:test";
import assert from "node:assert/strict";
import { attachment, createChatAppHarness, message } from "./helpers/chatAppHarness.mjs";

let app;
before(async () => {
  app = await createChatAppHarness();
});
beforeEach(async () => {
  await app.mount();
});
afterEach(async () => {
  await app.unmount();
});
after(async () => {
  await app.dispose();
});

async function originalRequest() {
  await app.type("failed A prompt");
  await app.addFile(attachment("original"));
  return app.send();
}
function assertDraft(prompt, files) {
  assert.equal(app.props.prompt, prompt);
  assert.deepEqual(app.props.attachments, files.map(attachment));
}

test("an unchanged source draft is restored with its attachments on failure", async () => {
  const request = await originalRequest();
  assertDraft("", []);
  await app.fail(request);
  assertDraft("failed A prompt", ["original"]);
  assert(app.text.includes("fixture request failure"));
});

test("an attachment-only request restores its file without inventing prompt text", async () => {
  await app.addFile(attachment("original"));
  const request = await app.send();
  await app.fail(request);
  assertDraft("", ["original"]);
});

test("typing a newer prompt in A prevents restoring A's old prompt and attachments", async () => {
  const request = await originalRequest();
  await app.type("newer A draft");
  await app.fail(request);
  assertDraft("newer A draft", []);
});

test("explicitly clearing the already-empty composer keeps it empty after late failure", async () => {
  const request = await originalRequest();
  await app.type("");
  await app.fail(request);
  assertDraft("", []);
});

test("adding a new attachment without typing protects the entire newer draft", async () => {
  const request = await originalRequest();
  await app.addFile(attachment("newer"));
  await app.fail(request);
  assertDraft("", ["newer"]);
});

test("adding then removing an attachment still prevents old file resurrection", async () => {
  const request = await originalRequest();
  await app.addFile(attachment("removed"));
  await app.removeFile("removed");
  await app.fail(request);
  assertDraft("", []);
});

test("a delayed A failure preserves B's text, attachments and error surface", async () => {
  const request = await originalRequest();
  await app.open("B");
  await app.type("unsent B draft");
  await app.addFile(attachment("B-file"));
  await app.fail(request);
  assert.equal(app.props.conversation.id, "B");
  assertDraft("unsent B draft", ["B-file"]);
  assert(!app.text.includes("fixture request failure"));
  await app.open("A");
  assert(app.text.includes("fixture request failure"));
});

test("A to B to A navigation invalidates rollback even without further typing", async () => {
  const request = await originalRequest();
  await app.open("B");
  await app.open("A");
  await app.fail(request);
  assertDraft("", []);
});

test("a newer draft survives A to B to A navigation and late failure", async () => {
  const request = await originalRequest();
  await app.open("B");
  await app.open("A");
  await app.type("later A draft");
  await app.addFile(attachment("later"));
  await app.fail(request);
  assertDraft("later A draft", ["later"]);
});

test("New chat keeps an intentionally empty composer empty", async () => {
  const request = await originalRequest();
  await app.newChat();
  await app.fail(request);
  assert.equal(app.props.conversation, null);
  assertDraft("", []);
  assert(!app.text.includes("fixture request failure"));
});

test("New chat preserves its newer prompt and file when A fails", async () => {
  const request = await originalRequest();
  await app.newChat();
  await app.type("new chat draft");
  await app.addFile(attachment("new-chat"));
  await app.fail(request);
  assert.equal(app.props.conversation, null);
  assertDraft("new chat draft", ["new-chat"]);
});

test("editing another prompt while A runs does not lose the edit target or newer text", async () => {
  const request = await originalRequest();
  await app.edit("A-user", "new edit draft");
  await app.fail(request);
  assertDraft("new edit draft", []);
  assert(app.text.includes("Editing opens a new chat"));
});

test("a failed edit fork keeps the original edit recoverable", async () => {
  await app.edit("A-user", "edited A prompt");
  const request = await app.send();
  await app.fail(request, app.fixture.fork);
  assertDraft("edited A prompt", []);
  assert(app.text.includes("Editing opens a new chat"));
  assert(
    !app.fixture.calls.some(([name]) => name === "sendChatMessage" || name === "deleteMessage"),
  );
});

test("a delayed fork failure cannot restore A's edit target into B", async () => {
  await app.edit("A-user", "edited A prompt");
  const request = await app.send();
  await app.open("B");
  await app.type("B draft during fork");
  await app.addFile(attachment("B-fork"));
  await app.fail(request, app.fixture.fork);
  assertDraft("B draft during fork", ["B-fork"]);
  assert(!app.text.includes("Editing opens a new chat"));
  assert(!app.text.includes("fixture request failure"));
});

test("a delayed successful fork followed by failed inference leaves B's draft untouched", async () => {
  await app.edit("A-user", "edited A prompt");
  const request = await app.send();
  await app.open("B");
  await app.type("B draft after fork");
  await app.addFile(attachment("B-new"));
  await app.resolve(app.fixture.fork, {
    conversation: { id: "edited", title: "A (edited)" },
    messages: [],
  });
  assert(
    app.fixture.calls.some(([name, owner]) => name === "sendChatMessage" && owner === "edited"),
  );
  await app.fail(request);
  assert.equal(app.props.conversation.id, "B");
  assertDraft("B draft after fork", ["B-new"]);
});

test("a newly created chat can restore its unchanged draft after failed inference", async () => {
  await app.newChat();
  await app.type("first request");
  await app.addFile(attachment("first"));
  const request = await app.send();
  await app.resolve(app.fixture.create, { id: "created", title: "Created" });
  assert.equal(app.props.conversation.id, "created");
  await app.fail(request);
  assertDraft("first request", ["first"]);
});

test("late conversation-creation failure cannot replace a new B draft", async () => {
  await app.newChat();
  await app.type("request needing a chat");
  const request = await app.send();
  await app.open("B");
  await app.type("B draft during creation");
  await app.addFile(attachment("B-create"));
  await app.fail(request, app.fixture.create);
  assert.equal(app.props.conversation.id, "B");
  assertDraft("B draft during creation", ["B-create"]);
});

test("late image-artifact failure cannot replace B's draft after persistence succeeds", async () => {
  await app.type("image request");
  const request = await app.send("image");
  assert(app.fixture.calls.some(([name]) => name === "createGenerationArtifact"));
  await app.open("B");
  await app.type("B after image");
  await app.addFile(attachment("B-image"));
  await app.fail(request);
  assertDraft("B after image", ["B-image"]);
  assert(!app.text.includes("fixture request failure"));
});

test("successful completion also preserves a newer draft in B", async () => {
  const request = await originalRequest();
  await app.open("B");
  await app.type("B on success");
  await app.addFile(attachment("B-success"));
  await app.succeed(request);
  assertDraft("B on success", ["B-success"]);
  assert.deepEqual(app.props.messages, [message("B")]);
});
