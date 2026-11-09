import test from "node:test";
import assert from "node:assert/strict";
import { ConversationState } from "../src/lib/conversationState.ts";

function deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}
const message = (owner, id, content = id, role = "user", status = "completed") => ({
  id,
  conversationId: owner,
  role,
  content,
  status,
  modelId: null,
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
});
const artifact = (owner, status = "generating") => ({
  id: owner + "-file",
  conversationId: owner,
  status,
});
const start = (owner, id = owner + "-reply") => ({
  conversationId: owner,
  user: message(owner, owner + "-user"),
  assistant: message(owner, id, "", "assistant", "streaming"),
});
const chunk = (owner, text, id = owner + "-reply") => ({
  conversationId: owner,
  messageId: id,
  chunk: text,
});
const done = (owner, id = owner + "-reply") => ({
  conversationId: owner,
  messageId: id,
  status: "completed",
});
const progress = (owner, id = owner + "-reply") => ({
  conversationId: owner,
  messageId: id,
  mode: "research",
  stage: "reading",
  completed: 1,
  total: 3,
  sources: 3,
  pagesRead: 1,
});
function selected(owner = "A") {
  const state = new ConversationState();
  state.selectConversation(owner);
  return state;
}

test("late message and artifact fetches never replace another conversation", async () => {
  const state = selected(),
    oldMessages = deferred(),
    oldArtifacts = deferred();
  const pending = [
    state.loadMessages("A", () => oldMessages.promise),
    state.loadArtifacts("A", () => oldArtifacts.promise),
  ];
  state.selectConversation("B");
  await state.loadMessages("B", async () => [message("B", "B-history"), message("A", "foreign")]);
  await state.loadArtifacts("B", async () => [artifact("B"), artifact("A")]);
  oldMessages.resolve([message("A", "A-history")]);
  oldArtifacts.resolve([artifact("A")]);
  await Promise.all(pending);
  assert.deepEqual(
    state.getSnapshot().messages.map((m) => m.id),
    ["B-history"],
  );
  assert.deepEqual(
    state.getSnapshot().artifacts.map((a) => a.id),
    ["B-file"],
  );
});

test("A to B to A navigation rejects the first A view epoch", async () => {
  const state = selected(),
    gate = deferred();
  const pending = state.loadMessages("A", () => gate.promise);
  state.selectConversation("B");
  state.selectConversation("A");
  await state.loadMessages("A", async () => [message("A", "fresh")]);
  gate.resolve([message("A", "stale")]);
  await pending;
  assert.equal(state.getSnapshot().messages[0].id, "fresh");
});

test("newest read wins independently for messages and artifacts", async () => {
  const state = selected(),
    older = deferred(),
    art = deferred();
  const pending = [
    state.loadMessages("A", () => older.promise),
    state.loadArtifacts("A", () => art.promise),
  ];
  await state.loadMessages("A", async () => [message("A", "new")]);
  older.resolve([message("A", "old")]);
  art.resolve([artifact("A", "ready")]);
  await Promise.all(pending);
  assert.equal(state.getSnapshot().messages[0].id, "new");
  assert.equal(state.getSnapshot().artifacts[0].status, "ready");
});

test("stream start, chunks and completion win over an older history snapshot", async () => {
  const state = selected(),
    gate = deferred();
  const pending = state.loadMessages("A", () => gate.promise);
  state.startStream(start("A"));
  state.streamChunk(chunk("A", "new text"));
  state.finishStream(done("A"));
  gate.resolve([message("A", "prefix"), message("A", "A-reply", "", "assistant", "streaming")]);
  await pending;
  assert.deepEqual(
    state
      .getSnapshot()
      .messages.map((m) => m.id)
      .sort(),
    ["A-reply", "A-user", "prefix"],
  );
  const reply = state.getSnapshot().messages.find((m) => m.id === "A-reply");
  assert.equal(reply.content, "new text");
  assert.equal(reply.status, "completed");
  assert.equal(state.run("A").streamingId, null);
});

test("pending reads cannot resurrect deleted messages or downgrade completed artifacts", async () => {
  const state = selected(),
    messages = deferred(),
    artifacts = deferred();
  state.setMessages("A", [message("A", "deleted")]);
  const pending = [
    state.loadMessages("A", () => messages.promise),
    state.loadArtifacts("A", () => artifacts.promise),
  ];
  state.setMessages("A", []);
  state.setArtifacts("A", [artifact("A", "ready")]);
  messages.resolve([message("A", "deleted")]);
  artifacts.resolve([artifact("A")]);
  await Promise.all(pending);
  assert.deepEqual(state.getSnapshot().messages, []);
  assert.equal(state.getSnapshot().artifacts[0].status, "ready");
});

test("background chunks, progress, activity and artifacts remain owned by A while B is visible", () => {
  const state = selected("B");
  state.setMessages("B", [message("B", "B-history")]);
  state.startStream(start("A"));
  state.streamChunk(chunk("A", "background"));
  state.scope("A").setActivity((a) => ({ ...a, searching: true }));
  state.scope("A").setWebProgress(progress("A"));
  state.setArtifacts("A", [artifact("A")]);
  assert.deepEqual(
    state.getSnapshot().messages.map((m) => m.id),
    ["B-history"],
  );
  assert.deepEqual(state.getSnapshot().artifacts, []);
  assert.equal(state.run("B").activity.typing, false);
  assert.equal(state.run("B").webProgress, null);
  assert.equal(state.isBusy("B"), false);
  assert.equal(state.isBusy("A"), true);
  state.selectConversation("A");
  assert.equal(state.getSnapshot().messages[1].content, "background");
  assert.equal(state.run("A").streamingId, "A-reply");
  assert.equal(state.run("A").activity.searching, true);
  assert.equal(state.run("A").webProgress.messageId, "A-reply");
});

test("foreign and old completion events cannot clear the current generation", () => {
  const state = selected("B");
  state.startStream(start("A"));
  state.startStream(start("B"));
  state.finishStream(done("A"));
  assert.equal(state.run("B").streamingId, "B-reply");
  assert.equal(state.run("B").activity.typing, true);
  state.startStream(start("B", "new-reply"));
  state.finishStream(done("B"));
  assert.equal(state.run("B").streamingId, "new-reply");
  state.streamChunk(chunk("B", "keep", "new-reply"));
  assert.equal(state.getSnapshot().messages.find((m) => m.id === "new-reply").content, "keep");
});

test("duplicate starts do not erase chunks and terminal streams cannot restart", () => {
  const state = selected();
  state.startStream(start("A"));
  state.streamChunk(chunk("A", "kept"));
  assert.equal(state.startStream(start("A")), false);
  assert.equal(state.getSnapshot().messages[1].content, "kept");
  state.finishStream(done("A"));
  assert.equal(state.startStream(start("A")), false);
  state.streamChunk(chunk("A", "late"));
  assert.equal(state.getSnapshot().messages[1].content, "kept");
});

test("mismatched started payloads and cross-owner scoped writes are ignored", () => {
  const state = selected("B");
  const event = start("A");
  event.assistant.conversationId = "B";
  assert.equal(state.startStream(event), false);
  assert.equal(state.anyBusy(), false);
  const scope = state.scope("A");
  scope.setMessages([message("A", "wrong")]);
  scope.setArtifacts([artifact("A")]);
  assert.deepEqual(state.getSnapshot().messages, []);
  assert.deepEqual(state.getSnapshot().artifacts, []);
});

test("web progress validates conversation and stream IDs and ignores late progress", () => {
  const state = selected();
  state.startStream(start("A"));
  state.scope("A").setWebProgress(progress("B"));
  assert.equal(state.run("A").webProgress, null);
  state.scope("A").setWebProgress(progress("A", "wrong"));
  assert.equal(state.run("A").webProgress, null);
  state.scope("A").setWebProgress(progress("A"));
  assert.equal(state.run("A").webProgress.stage, "reading");
  state.finishStream(done("A"));
  state.scope("A").setWebProgress(progress("A"));
  assert.equal(state.run("A").webProgress, null);
});

test("request leases synchronously enforce the shared runtime limit across chats", () => {
  const state = selected();
  const lease = state.beginRequest("A");
  assert(lease);
  state.selectConversation("B");
  assert.equal(state.beginRequest("B"), null);
  state.finishRequest(lease);
  const next = state.beginRequest("B");
  assert(next);
  state.finishRequest(lease);
  assert.equal(state.run("B").request, next.token);
});

test("assigning a newly created chat keeps the current view and transfers request ownership", () => {
  const state = new ConversationState();
  const view = state.captureView();
  const lease = state.beginRequest(null);
  state.selectConversation("B");
  state.assignRequest(lease, "created");
  assert.equal(state.isCurrent(view), false);
  assert.equal(state.getSnapshot().activeId, "B");
  assert.equal(state.isBusy(null), false);
  assert.equal(state.run("created").request, lease.token);
  state.startStream(start("created"));
  state.finishRequest(lease);
  assert.equal(state.isBusy("created"), false);
  assert.equal(state.isBusy("B"), false);
});

test("request completion cannot reset a newer native stream or another chat", () => {
  const state = selected();
  const lease = state.beginRequest("A");
  state.startStream(start("A"));
  state.finishStream(done("A"));
  state.startStream(start("A", "new-reply"));
  state.startStream(start("B"));
  state.finishRequest(lease);
  assert.equal(state.run("A").streamingId, "new-reply");
  assert.equal(state.run("B").streamingId, "B-reply");
});

test("active failures surface, but stale reads and stale failures have no visible effect", async () => {
  const state = selected(),
    gate = deferred();
  const pending = state.loadMessages("A", () => gate.promise);
  state.selectConversation("B");
  gate.reject(Error("stale"));
  await pending;
  await assert.rejects(
    state.loadMessages("B", async () => {
      throw Error("current");
    }),
    /current/,
  );
  let fetched = false;
  await state.loadMessages("A", async () => {
    fetched = true;
    return [];
  });
  assert.equal(fetched, false);
});

test("loading a running history reconstructs Stop ownership without foreign rows", async () => {
  const state = selected();
  await state.loadMessages("A", async () => [
    message("A", "system", "", "system"),
    start("A").assistant,
    start("B").assistant,
  ]);
  assert.equal(state.run("A").streamingId, "A-reply");
  assert.equal(state.isBusy("B"), false);
  assert.deepEqual(
    state.getSnapshot().messages.map((m) => m.id),
    ["A-reply"],
  );
  state.selectConversation("B");
  state.streamChunk(chunk("A", "background continuation"));
  state.selectConversation("A");
  assert.equal(state.getSnapshot().messages[0].content, "background continuation");
});

test("optimistic requests survive stale reads but are deduplicated by native history", async () => {
  const state = selected();
  const lease = state.beginRequest("A");
  state.setMessages("A", [message("A", "local-draft", "new prompt")]);
  await state.loadMessages("A", async () => []);
  assert.equal(state.getSnapshot().messages.length, 1);
  await state.loadMessages("A", async () => [message("A", "native-user", "new prompt")]);
  assert.deepEqual(
    state.getSnapshot().messages.map((m) => m.id),
    ["native-user"],
  );
  state.finishRequest(lease);
});

test("subscriptions publish immutable snapshots and unsubscribe cleanly", () => {
  const state = selected();
  const before = state.getSnapshot();
  let updates = 0;
  const unsubscribe = state.subscribe(() => {
    updates += 1;
  });
  state.startStream(start("A"));
  assert.notStrictEqual(state.getSnapshot(), before);
  assert.equal(before.messages.length, 0);
  assert.equal(before.runs.size, 0);
  assert(updates > 0);
  unsubscribe();
  const count = updates;
  state.finishStream(done("A"));
  assert.equal(updates, count);
});

test("Stop in idle B never cancels A; background Stop and returning to A target A", async () => {
  const state = selected();
  const calls = [];
  const cancel = async (owner) => {
    calls.push(owner);
  };
  state.startStream(start("A"));
  state.selectConversation("B");
  await state.cancelActiveGeneration(cancel);
  assert.deepEqual(calls, []);
  await state.cancelGeneration("A", cancel);
  assert.deepEqual(calls, ["A"]);
  state.selectConversation("A");
  await state.cancelActiveGeneration(cancel);
  assert.deepEqual(calls, ["A", "A"]);
  state.finishStream(done("A"));
  await state.cancelActiveGeneration(cancel);
  assert.deepEqual(calls, ["A", "A"]);
});

test("Stop captures its owner before awaiting cancellation and does not reset another run", async () => {
  const state = selected();
  const gate = deferred();
  const calls = [];
  state.startStream(start("A"));
  const pending = state.cancelActiveGeneration(async (owner) => {
    calls.push(owner);
    await gate.promise;
  });
  state.selectConversation("B");
  state.startStream(start("B"));
  gate.resolve();
  await pending;
  assert.deepEqual(calls, ["A"]);
  assert.equal(state.run("B").streamingId, "B-reply");
  await assert.rejects(
    state.cancelActiveGeneration(async () => {
      throw Error("cancel failed");
    }),
    /cancel failed/,
  );
  assert.equal(state.run("B").streamingId, "B-reply");
});

test("a new stream clears progress for an older message and retains its own pre-start progress", () => {
  const state = selected();
  state.startStream(start("A"));
  state.scope("A").setWebProgress(progress("A"));
  state.startStream(start("A", "new-reply"));
  assert.equal(state.run("A").webProgress, null);
  const other = selected();
  other.scope("A").setWebProgress(progress("A"));
  other.startStream(start("A"));
  assert.equal(other.run("A").webProgress.messageId, "A-reply");
});

test("an edit fork transfers request and Stop ownership while another chat remains visible", async () => {
  const state = selected();
  state.setMessages("A", [message("A", "original")]);
  const lease = state.beginRequest("A");
  state.selectConversation("B");
  state.setMessages("B", [message("B", "B-history")]);
  state.assignRequest(lease, "edited");
  state.startStream(start("edited"));
  const calls = [];
  const cancel = async (owner) => {
    calls.push(owner);
  };
  await state.cancelGeneration("A", cancel);
  await state.cancelActiveGeneration(cancel);
  await state.cancelGeneration("edited", cancel);
  assert.deepEqual(calls, ["edited"]);
  assert.equal(state.isBusy("A"), false);
  assert.equal(state.getSnapshot().activeId, "B");
  assert.deepEqual(
    state.getSnapshot().messages.map((row) => row.id),
    ["B-history"],
  );
  state.finishRequest(lease);
  assert.equal(state.isBusy("edited"), false);
});
