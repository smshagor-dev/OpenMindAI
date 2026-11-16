import test, { before, beforeEach, after, afterEach } from "node:test";
import assert from "node:assert/strict";
import { act } from "react";
import { attachment, createChatAppHarness, deferred } from "./helpers/chatAppHarness.mjs";
let app;
before(async () => { app = await createChatAppHarness(); });
beforeEach(async () => { await app.mount(); });
afterEach(async () => { await app.unmount(); });
after(async () => { await app.dispose(); });
function files(prefix, count, kind = "text") {
  return Array.from({ length: count }, (_, index) => {
    const attachmentDraft = { ...attachment(`${prefix}-${index}`), kind };
    // Browser File objects always have name/size/type; the old mock omitted
    // the name and falsely failed the PDF import progress assertion.
    return { name: attachmentDraft.name, size: attachmentDraft.size, type: attachmentDraft.type, attachmentDraft };
  });
}
async function start(selection) {
  let pending; await act(async () => { pending = app.props.addFiles(selection); }); return { pending };
}
async function release(gate, ...requests) { await act(async () => { gate.resolve(); await Promise.all(requests.map(r=>r.pending)); }); }

test("overlapping six-file selections remain capped at eight and excess files are not decoded", async () => {
  const first = files("first", 6), second = files("second", 6), gate = deferred(); first[0].gate = gate;
  const one = await start(first), two = await start(second);
  assert.equal(first[0].reads, 1); assert.equal(second[0].reads, undefined);
  await release(gate, one, two);
  assert.equal(app.props.attachments.length, 8);
  assert.equal(second[2].reads, undefined);
  assert(app.text.includes("at most 8 attachments"));
});
test("overlapping image selections remain capped at four without blocking later text", async () => {
  const first = files("one", 3, "image"), second = files("two", 3, "image"), gate = deferred(); first[0].gate=gate;
  const one=await start(first),two=await start(second); await release(gate,one,two);
  assert.equal(app.props.attachments.filter(x=>x.kind==="image").length,4);
  assert.equal(second[1].reads,undefined);
  await app.addFile(attachment("text")); assert.equal(app.props.attachments.length,5);
});
test("failed/unsupported files do not consume slots and the queue permits retry", async () => {
  const bad = files("bad",1)[0]; bad.failure=Error("fixture decode failure");
  const unsupported={kind:"binary"}; const good=files("good",1);
  await act(async()=>app.props.addFiles([bad,unsupported,...good]));
  assert.equal(unsupported.reads,undefined); assert.equal(app.props.attachments.length,1);
  delete bad.failure; await act(async()=>app.props.addFiles([bad]));
  assert.equal(app.props.attachments.length,2); assert.equal(bad.reads,2);
});
test("removal while decoding frees capacity used by the queued selection", async () => {
  await act(async()=>app.props.addFiles(files("existing",7)));
  const gate=deferred(),incoming=files("incoming",2);incoming[0].gate=gate;
  const request=await start(incoming);await app.removeFile("existing-0"); await release(gate,request);
  assert.equal(app.props.attachments.length,8);
  assert(!app.props.attachments.some(x=>x.id==="existing-0"));
  assert(app.props.attachments.some(x=>x.id==="incoming-1"));
});
test("Cancel import aborts in-flight PDF work and discards its decoded attachment", async () => {
  const pending = deferred();
  const incoming = files("cancel-pdf", 1, "pdf");
  incoming[0].gate = pending;
  const running = await start(incoming);
  assert.equal(incoming[0].reads, 1);
  assert.equal(incoming[0].readOptions.signal.aborted, false);
  assert.equal(app.props.pdfImportProgress.fileName, incoming[0].attachmentDraft.name);
  await act(async () => app.props.onCancelPdfImport());
  assert.equal(incoming[0].readOptions.signal.aborted, true);
  assert.equal(app.props.pdfImportProgress, null);
  await release(pending, running);
  assert.equal(app.props.attachments.length, 0, "cancelled PDF must never be committed");
});

test("opening another chat aborts pending PDF ingestion before it can populate the new chat", async () => {
  const pending = deferred();
  const incoming = files("previous-chat-pdf", 1, "pdf");
  incoming[0].gate = pending;
  const running = await start(incoming);
  assert.equal(incoming[0].readOptions.signal.aborted, false);
  await app.open("B");
  assert.equal(incoming[0].readOptions.signal.aborted, true);
  await release(pending, running);
  assert.equal(app.props.attachments.length, 0);
});

test("leaving chat for Tools cancels expensive PDF OCR instead of leaving background CPU work", async () => {
  const gate = deferred();
  const incoming = files("tools-switch-pdf", 1, "pdf");
  incoming[0].gate = gate;
  const request = await start(incoming);
  assert.equal(incoming[0].readOptions.signal.aborted, false);
  await app.openTools();
  assert.equal(incoming[0].readOptions.signal.aborted, true);
  await release(gate, request);
  assert.equal(incoming[0].reads, 1);
});

test("navigation and explicit New chat discard both in-flight and queued old selections", async () => {
  const gate=deferred(),first=files("old",1),second=files("queued",1);first[0].gate=gate;
  const one=await start(first),two=await start(second);
  await app.open("B"); await release(gate,one,two);
  assert.equal(app.props.attachments.length,0);assert.equal(second[0].reads,undefined);
  await app.newChat(); const nextGate=deferred(),next=files("new",1);next[0].gate=nextGate;
  const pending=await start(next); await app.newChat();await release(nextGate,pending);
  assert.equal(app.props.attachments.length,0);
});
