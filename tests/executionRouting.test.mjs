import test from "node:test";
import assert from "node:assert/strict";
import {
  selectExecutionRoute, isRecoverableCloudFailure, runWithFallback,
  shouldShowCloudModels, cloudProbeDelayMs, requestsLocalOnly, offlineSourceNotice, ResponsePersistenceError,
} from "../src/lib/executionRouting.ts";
import { beginSetupPreparation, setupExecutionMode, setupPreparationPending, completeSetupPreparation } from "../src/lib/setupExecutionMode.ts";

test("auto routing prefers Cloud when authenticated and online", () => {
  assert.equal(selectExecutionRoute({preference:"auto",task:"chat",online:true,cloudConnected:true,cloudModelReady:true}).target,"cloud");
  assert.equal(selectExecutionRoute({preference:"auto",task:"research",online:true,cloudConnected:true}).target,"cloud");
  assert.equal(selectExecutionRoute({preference:"auto",task:"scene3d",online:true,cloudConnected:true}).target,"cloud");
});

test("offline or disconnected Cloud always routes supported work locally", () => {
  for (const task of ["chat","vision","search","research","scene3d"]) {
    const offline = selectExecutionRoute({preference:"auto",task,online:false,cloudConnected:true});
    assert.equal(offline.target,"local",task);
    assert.equal(offline.offline,true);
    assert.equal(selectExecutionRoute({preference:"cloud",task,online:true,cloudConnected:false}).target,"deferred");
  }
});

test("explicit Local preference retains local processing when online", () => {
  assert.equal(selectExecutionRoute({preference:"local",task:"chat",online:true,cloudConnected:true}).target,"local");
  assert.equal(selectExecutionRoute({preference:"auto",task:"chat",online:true,cloudConnected:true,cloudModelReady:false}).target,"local");
});

test("never claim local image-to-mesh parity for an uploaded reference", () => {
  assert.equal(selectExecutionRoute({preference:"auto",task:"image-to-3d",online:false,cloudConnected:true}).target,"deferred");
  assert.equal(selectExecutionRoute({preference:"local",task:"image-to-3d",online:true,cloudConnected:false}).target,"deferred");
  assert.equal(selectExecutionRoute({preference:"local",task:"image-to-3d",online:true,cloudConnected:true}).target,"deferred");
});

test("Cloud network failures permit one local fallback without remote replay", async () => {
  let cloudCalls=0, localCalls=0;
  const decision=selectExecutionRoute({preference:"auto",task:"chat",online:true,cloudConnected:true});
  const value=await runWithFallback(decision,async()=>{cloudCalls++;throw new Error("Cloud/Studio: Cloud API request failed");},async()=>{localCalls++;return "local answer";});
  assert.deepEqual(value,{result:"local answer",target:"local",fallback:true});
  assert.equal(cloudCalls,1);
  assert.equal(localCalls,1);
});

test("a throwing fallback notification cannot prevent local recovery", async () => {
  let remoteCalls = 0;
  let localCalls = 0;
  let notifications = 0;
  const decision = selectExecutionRoute({ preference: "auto", task: "chat", online: true, cloudConnected: true });
  const result = await runWithFallback(
    decision,
    async () => { remoteCalls++; throw new Error("HTTP 503: unavailable"); },
    async () => { localCalls++; return "recovered"; },
    () => { notifications++; throw new Error("notification UI unmounted"); },
  );
  assert.deepEqual(result, { result: "recovered", target: "local", fallback: true });
  assert.equal(remoteCalls, 1, "never replay a remote POST");
  assert.equal(localCalls, 1, "recover locally exactly once");
  assert.equal(notifications, 1);
});

test("a throwing fallback notification is not called for rejected Cloud failures", async () => {
  let notifications = 0;
  let localCalls = 0;
  await assert.rejects(runWithFallback(
    { target: "cloud", offline: false, reason: "Cloud", fallbackAllowed: true },
    async () => { throw new ResponsePersistenceError("database locked"); },
    async () => { localCalls++; return "unexpected"; },
    () => { notifications++; throw new Error("should not run"); },
  ), ResponsePersistenceError);
  assert.equal(notifications, 0);
  assert.equal(localCalls, 0);
});

test("validation and response-persistence errors are never replayed", async()=>{
  for (const error of [new Error("Cloud/Studio: HTTP 422: invalid model"),new ResponsePersistenceError("SQLite locked")]) {
    let localCalls=0;
    await assert.rejects(runWithFallback({target:"cloud",offline:false,reason:"Connected",fallbackAllowed:true},async()=>{throw error;},async()=>{localCalls++;return "wrong";}));
    assert.equal(localCalls,0);
  }
});

test("offline web search cannot claim verified live sources",()=>{
  const result=offlineSourceNotice("Is this headline true?");
  assert.match(result,/internet is unavailable/);
  assert.match(result,/Do not claim that live web search/);
  assert.ok(result.endsWith("Is this headline true?"));
  assert.equal(isRecoverableCloudFailure("HTTP 401: session expired"),true);
  assert.equal(isRecoverableCloudFailure("HTTP 503: unavailable"),true);
  assert.equal(isRecoverableCloudFailure("Cloud returned an empty assistant response."), true);
  assert.equal(isRecoverableCloudFailure("Server returned non-JSON response (403)"), true);
});

test("the API model selector appears only when online and Cloud is authenticated", () => {
  for (const mode of ["auto", "cloud"]) {
    assert.equal(shouldShowCloudModels(mode, true, true, 3), true, mode);
    assert.equal(shouldShowCloudModels(mode, false, true, 3), false, "offline");
    assert.equal(shouldShowCloudModels(mode, true, false, 3), false, "signed out");
    assert.equal(shouldShowCloudModels(mode, true, true, 0), false, "no hosted models");
  }
  assert.equal(shouldShowCloudModels("local", true, true, 2), false, "Local always shows local models");
});

test("Cloud model selector rejects malformed discovery counts", () => {
  for (const count of [NaN, Infinity, -Infinity, -1, -0.5, 0.5, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
    assert.equal(shouldShowCloudModels("auto", true, true, count), false, String(count));
  }
  for (const count of [1, 2, Number.MAX_SAFE_INTEGER]) {
    assert.equal(shouldShowCloudModels("auto", true, true, count), true, String(count));
  }
  assert.equal(shouldShowCloudModels("cloud", true, true, 1), true);
  assert.equal(shouldShowCloudModels("local", true, true, 1), false);
  assert.equal(shouldShowCloudModels("auto", false, true, 1), false);
  assert.equal(shouldShowCloudModels("auto", true, false, 1), false);
});

test("explicit offline-only prompts override authenticated Cloud, including multilingual input", () => {
  for (const prompt of [
    "This is sensitive; offline only, do not use cloud",
    "শুধু লোকাল মডেল ব্যবহার করো",
    "ইন্টারনেট ছাড়া কাজ করো",
    "Только локально, пожалуйста",
  ]) {
    assert.equal(requestsLocalOnly(prompt), true, prompt);
    const route = selectExecutionRoute({
      preference: "auto", task: "research", online: true, cloudConnected: true,
      cloudModelReady: true, forceLocal: requestsLocalOnly(prompt),
    });
    assert.equal(route.target, "local", prompt);
  }
  assert.equal(requestsLocalOnly("Research new online API design"), false);
  assert.equal(selectExecutionRoute({
    preference:"cloud", task:"image-to-3d", online:true, cloudConnected:true,
    forceLocal:true,
  }).target, "deferred", "Never upload local-only reference images");
});

test("mobile-style foreground Cloud recovery checks back off without repeated generation", () => {
  assert.equal(cloudProbeDelayMs(true, 0), 60000);
  assert.equal(cloudProbeDelayMs(false, 1), 5000);
  assert.equal(cloudProbeDelayMs(false, 2), 10000);
  assert.equal(cloudProbeDelayMs(false, 3), 20000);
  assert.equal(cloudProbeDelayMs(false, 4), 40000);
  assert.equal(cloudProbeDelayMs(false, 5), 60000);
  assert.equal(cloudProbeDelayMs(false, 20), 60000);
  assert.equal(cloudProbeDelayMs(false, 1, 0.8), 4000);
  assert.equal(cloudProbeDelayMs(false, 1, 1.2), 6000);
});

test("Cloud health probe retry delays stay finite for malformed telemetry", () => {
  for (const failures of [NaN, Infinity, -Infinity]) {
    assert.equal(cloudProbeDelayMs(false, failures), 5000);
  }
  for (const jitter of [NaN, Infinity, -Infinity]) {
    assert.equal(cloudProbeDelayMs(false, 3, jitter), 20000);
  }
  assert.equal(cloudProbeDelayMs(false, -12), 5000);
  assert.equal(cloudProbeDelayMs(false, 2.9), 10000);
  assert.equal(cloudProbeDelayMs(false, 4, -10), 32000);
  assert.equal(cloudProbeDelayMs(false, 4, 10), 48000);
  assert.equal(cloudProbeDelayMs(true, NaN, NaN), 60000);
});

test("Cloud-only runs no Local fallback even after a network error", async () => {
  const route = selectExecutionRoute({preference:"cloud", task:"chat", online:true, cloudConnected:true});
  assert.equal(route.target,"cloud");
  assert.equal(route.fallbackAllowed,false);
  let localCalls = 0;
  await assert.rejects(runWithFallback(route, async () => {
    throw new Error("Cloud API request failed");
  }, async () => { localCalls++; return "wrong"; }), /Cloud API request failed/);
  assert.equal(localCalls,0);
});

test("first-run Cloud/Local/Both choice survives simulated relaunch before downloads", () => {
  const originalWindow = globalThis.window;
  const storage = new Map();
  globalThis.window = { localStorage: {
    getItem: (key) => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, String(value)),
    removeItem: (key) => storage.delete(key),
  } };
  try {
    for (const mode of ["cloud", "local", "auto"]) {
      beginSetupPreparation(mode);
      assert.equal(setupExecutionMode(), mode);
      assert.equal(setupPreparationPending(), true);
      completeSetupPreparation();
      assert.equal(setupPreparationPending(), false);
      assert.equal(setupExecutionMode(), mode);
    }
    storage.set("openmindai.setup.executionMode.v1", "invalid");
    assert.equal(setupExecutionMode(), null, "never trust an invalid persisted execution mode");
    assert.equal(setupPreparationPending(), false);
  } finally {
    if (originalWindow === undefined) delete globalThis.window;
    else globalThis.window = originalWindow;
  }
});

test("hostile Cloud error metadata cannot interrupt permitted Local fallback", async () => {
  const throwingMessage = Object.defineProperty({}, "message", {
    get() { throw new Error("accessor exploded"); },
  });
  const revoked = Proxy.revocable({}, {});
  revoked.revoke();
  const throwingCoercion = { message: { toString() { throw new Error("coercion exploded"); } } };
  for (const error of [throwingMessage, revoked.proxy, throwingCoercion]) {
    let localCalls = 0;
    const result = await runWithFallback(
      { target: "cloud", offline: false, reason: "connected", fallbackAllowed: true },
      async () => { throw error; },
      async () => { localCalls++; return "local"; },
    ).catch((caught) => {
      assert.equal(caught, error, "unknown failures should propagate unchanged");
      return null;
    });
    assert.equal(result, null);
    assert.equal(localCalls, 0, "unknown errors must not trigger local replay");
  }
});

test("Cloud failure classification safely handles throwing getters and revoked proxies", () => {
  const getter = Object.defineProperty({}, "message", {
    get() { throw new Error("private diagnostic"); },
  });
  const revoked = Proxy.revocable({}, {});
  revoked.revoke();
  for (const error of [getter, revoked.proxy, { message: Symbol("private") }]) {
    assert.equal(isRecoverableCloudFailure(error), false);
  }
  assert.equal(isRecoverableCloudFailure({ message: "HTTP 503: unavailable" }), true);
});

test("explicit device retention and never-upload instructions override Cloud routing", () => {
  const privatePrompts = [
    "Keep this on my device, please.",
    "Store everything on this computer.",
    "Process this only on my machine.",
    "Run on my PC, not remotely.",
    "Never upload this document to the cloud.",
    "Never share my data with a server.",
  ];
  for (const prompt of privatePrompts) {
    assert.equal(requestsLocalOnly(prompt), true, prompt);
    const decision = selectExecutionRoute({
      preference: "auto", task: "chat", online: true, cloudConnected: true,
      cloudModelReady: true, forceLocal: requestsLocalOnly(prompt),
    });
    assert.equal(decision.target, "local", prompt);
  }
  for (const prompt of [
    "How do I store a file on my device?",
    "Explain cloud servers and on-device AI.",
    "Never upload a file without validating its format.",
    "Run a benchmark on a remote machine.",
  ]) {
    assert.equal(requestsLocalOnly(prompt), false, prompt);
  }
  assert.equal(selectExecutionRoute({
    preference: "auto", task: "image-to-3d", online: true, cloudConnected: true,
    forceLocal: requestsLocalOnly("Keep this on my device"),
  }).target, "deferred", "Private requests must not silently use Cloud when local capability is absent");
});

test("auto Cloud routing does not attempt unavailable or unsupported Local fallback", async () => {
  for (const input of [
    { preference: "auto", task: "chat", online: true, cloudConnected: true, localAvailable: false },
    { preference: "auto", task: "image-to-3d", online: true, cloudConnected: true, localAvailable: true },
  ]) {
    const route = selectExecutionRoute(input);
    assert.equal(route.target, "cloud");
    assert.equal(route.fallbackAllowed, false);
    let remoteCalls = 0, localCalls = 0, notifications = 0;
    await assert.rejects(
      runWithFallback(route,
        async () => { remoteCalls++; throw new Error("Cloud API request failed"); },
        async () => { localCalls++; return "unexpected local output"; },
        () => { notifications++; }),
      /Cloud API request failed/,
    );
    assert.equal(remoteCalls, 1);
    assert.equal(localCalls, 0);
    assert.equal(notifications, 0);
  }
});

test("auto Cloud routing retains exactly one fallback for supported installed Local tasks", async () => {
  const route = selectExecutionRoute({
    preference: "auto", task: "vision", online: true, cloudConnected: true, localAvailable: true,
  });
  assert.equal(route.fallbackAllowed, true);
  let remoteCalls = 0, localCalls = 0;
  const response = await runWithFallback(route,
    async () => { remoteCalls++; throw new Error("Cloud API request failed"); },
    async () => { localCalls++; return "local response"; });
  assert.deepEqual(response, { result: "local response", target: "local", fallback: true });
  assert.equal(remoteCalls, 1);
  assert.equal(localCalls, 1);
});

test("Cloud recovery requires explicit fallback permission, never an omitted policy", async () => {
  for (const fallbackAllowed of [undefined, false]) {
    let remoteCalls = 0, localCalls = 0, notifications = 0;
    const decision = { target: "cloud", offline: false, reason: "Cloud execution", fallbackAllowed };
    await assert.rejects(
      runWithFallback(
        decision,
        async () => { remoteCalls++; throw new Error("Cloud API request failed"); },
        async () => { localCalls++; return "unexpected local response"; },
        () => { notifications++; },
      ),
      /Cloud API request failed/,
    );
    assert.equal(remoteCalls, 1, "never replay the remote request");
    assert.equal(localCalls, 0, "missing or denied permission must not start local inference");
    assert.equal(notifications, 0, "do not announce a fallback that cannot happen");
  }

  let localCalls = 0;
  const optedIn = { target: "cloud", offline: false, reason: "Auto recovery", fallbackAllowed: true };
  const recovered = await runWithFallback(
    optedIn,
    async () => { throw new Error("Cloud API request failed"); },
    async () => { localCalls++; return "recovered locally"; },
  );
  assert.deepEqual(recovered, { result: "recovered locally", target: "local", fallback: true });
  assert.equal(localCalls, 1);
});

test("invalid runtime execution selectors defer without contacting either engine", async () => {
  const valid = { preference: "auto", task: "chat", online: true, cloudConnected: true };
  for (const invalid of [
    { preference: "automatic" }, { preference: "" }, { preference: null },
    { preference: 0 }, { task: "unknown" }, { task: null }, { task: "" },
  ]) {
    const decision = selectExecutionRoute({ ...valid, ...invalid });
    assert.equal(decision.target, "deferred", JSON.stringify(invalid));
    assert.equal(decision.fallbackAllowed, false);
    let remoteCalls = 0, localCalls = 0;
    await assert.rejects(
      runWithFallback(decision, async () => { remoteCalls++; return "remote"; },
        async () => { localCalls++; return "local"; }),
      /Invalid execution mode or task/,
    );
    assert.equal(remoteCalls, 0);
    assert.equal(localCalls, 0);
  }
  assert.equal(selectExecutionRoute(valid).target, "cloud", "valid Auto behavior is unchanged");
  assert.equal(selectExecutionRoute({ ...valid, preference: "local" }).target, "local");
  assert.equal(selectExecutionRoute({ ...valid, preference: "cloud" }).target, "cloud");
});

test("hostile Cloud error metadata fails closed without local replay", async () => {
  const revoked = Proxy.revocable({}, {});
  revoked.revoke();
  const throwingGetter = Object.defineProperty({}, "message", {
    get() { throw new Error("metadata getter failed"); },
  });
  const throwingCoercion = { message: { toString() { throw new Error("coercion failed"); } } };
  const decision = { target: "cloud", reason: "auto", offline: false, fallbackAllowed: true };
  for (const error of [revoked.proxy, throwingGetter, throwingCoercion]) {
    assert.equal(isRecoverableCloudFailure(error), false);
    let localCalls = 0;
    const remote = async () => { throw error; };
    await assert.rejects(
      runWithFallback(decision, remote, async () => { localCalls++; return "unsafe replay"; }),
      (received) => received === error,
    );
    assert.equal(localCalls, 0);
  }
});

test("normal HTTP 503 still permits exactly one local recovery", async () => {
  const decision = { target: "cloud", reason: "auto", offline: false, fallbackAllowed: true };
  let remoteCalls = 0, localCalls = 0;
  const result = await runWithFallback(
    decision,
    async () => { remoteCalls++; throw new Error("HTTP 503 Service Unavailable"); },
    async () => { localCalls++; return "recovered"; },
  );
  assert.deepEqual(result, { result: "recovered", target: "local", fallback: true });
  assert.equal(remoteCalls, 1);
  assert.equal(localCalls, 1);
});
