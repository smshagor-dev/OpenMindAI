import test from "node:test";
import assert from "node:assert/strict";
import { createUpdateCoordinator } from "../src/lib/updateCoordinator.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function fakeUpdate(version, download) {
  const state = { installs: 0, closes: 0 };
  return {
    version,
    state,
    downloadAndInstall: async (progress) => {
      state.installs += 1;
      await download(progress);
    },
    close: async () => { state.closes += 1; },
  };
}

test("StrictMode startup and overlapping focus/interval checks use one resource", async () => {
  const gate = deferred();
  let checks = 0;
  const update = fakeUpdate("2.0.0", async () => {});
  const coordinator = createUpdateCoordinator(async () => {
    checks += 1;
    await gate.promise;
    return update;
  });

  const one = coordinator.check();
  const two = coordinator.check();
  const three = coordinator.check();
  assert.strictEqual(one, two);
  assert.strictEqual(two, three);
  assert.equal(checks, 1);
  gate.resolve();
  const results = await Promise.all([one, two, three]);
  assert.ok(results.every((result) => result.version === "2.0.0"));
  assert.equal(update.state.closes, 0);
});

test("manual and automatic installs share one download, progress and ready state", async () => {
  const gate = deferred();
  const update = fakeUpdate("2.1.0", async (progress) => {
    progress({ event: "Started", contentLength: 100 });
    progress({ event: "Progress", chunkLength: 25 });
    await gate.promise;
    progress({ event: "Progress", chunkLength: 75 });
  });
  const coordinator = createUpdateCoordinator(async () => update);
  const events = [];
  const unsubscribe = coordinator.subscribe((next) => events.push(next));
  await coordinator.check();

  const automatic = coordinator.downloadAndInstall();
  const manual = coordinator.downloadAndInstall();
  assert.strictEqual(automatic, manual);
  assert.equal(update.state.installs, 1);
  assert.equal(coordinator.getSnapshot().state, "downloading");
  assert.equal(coordinator.getSnapshot().downloadedBytes, 25);
  assert.equal(coordinator.getSnapshot().totalBytes, 100);
  // Focus and interval checks during an install may not replace the resource.
  await coordinator.check();
  gate.resolve();
  await Promise.all([automatic, manual]);
  assert.equal(update.state.installs, 1);
  assert.equal(update.state.closes, 1);
  assert.equal(coordinator.getSnapshot().state, "ready");
  assert.ok(events.some((event) => event.downloadedBytes === 100));
  await coordinator.downloadAndInstall();
  assert.equal(update.state.installs, 1);
  unsubscribe();
});

test("manual install while startup check is pending reuses the pending check", async () => {
  const gate = deferred();
  let checks = 0;
  const update = fakeUpdate("3.0.0", async () => {});
  const coordinator = createUpdateCoordinator(async () => {
    checks += 1;
    await gate.promise;
    return update;
  });
  const startup = coordinator.check();
  const manual = coordinator.downloadAndInstall();
  gate.resolve();
  await Promise.all([startup, manual]);
  assert.equal(checks, 1);
  assert.equal(update.state.installs, 1);
  assert.equal(coordinator.getSnapshot().state, "ready");
});

test("failed version retries with a fresh handle and does not reuse a failed installer", async () => {
  let checks = 0;
  const failedUpdate = fakeUpdate("4.0.0", async () => { throw new Error("network disconnected"); });
  const recoveredUpdate = fakeUpdate("4.0.0", async () => {});
  const coordinator = createUpdateCoordinator(async () => {
    checks += 1;
    return checks === 1 ? failedUpdate : recoveredUpdate;
  });
  await coordinator.check();
  await assert.rejects(coordinator.downloadAndInstall(), /network disconnected/);
  assert.equal(coordinator.getSnapshot().state, "failed");
  assert.equal(coordinator.getSnapshot().version, "4.0.0");
  assert.equal(failedUpdate.state.closes, 1);
  await coordinator.downloadAndInstall();
  assert.equal(checks, 2);
  assert.equal(failedUpdate.state.installs, 1);
  assert.equal(recoveredUpdate.state.installs, 1);
  assert.equal(coordinator.getSnapshot().state, "ready");
});

test("no update never triggers download and closes a stale available version", async () => {
  let calls = 0;
  const update = fakeUpdate("5.0.0", async () => {});
  const coordinator = createUpdateCoordinator(async () => {
    calls += 1;
    return calls === 1 ? update : null;
  });
  await coordinator.check();
  const state = await coordinator.check();
  assert.equal(state.state, "up-to-date");
  assert.equal(update.state.installs, 0);
  assert.equal(update.state.closes, 1);
  await assert.rejects(coordinator.downloadAndInstall(), /No application update is available/);
});
