const assert = require("node:assert/strict");
const { test } = require("node:test");

const {
  DISABLED_MESSAGE,
  ensureCodingAgentReady,
  loadingMessage
} = require("../dist/agentReadiness.js");

const model = { id: "lite", name: "OpenAgent Lite", displayName: "NVIDIA Nemotron 3 Nano 4B" };

function info(state, extra = {}) {
  return {
    service: "openmindai-coding-agent",
    configured: true,
    codingEnabled: state !== "disabled",
    model,
    runtime: { state, ...extra }
  };
}

/** Replays a scripted desktop: each GET returns the next status; POST /start returns `afterStart`. */
function scriptedDesktop(statuses, afterStart) {
  const calls = [];
  let clock = 0;
  return {
    calls,
    options: {
      pollMs: 1000,
      timeoutMs: 330_000,
      now: () => clock,
      sleep: async (ms) => {
        clock += ms;
      },
      fetchJson: async (url, method, headers) => {
        calls.push({ url, method, headers });
        if (method === "POST") return afterStart;
        return statuses.length > 1 ? statuses.shift() : statuses[0];
      }
    }
  };
}

test("an already-ready agent returns immediately without starting anything", async () => {
  const desktop = scriptedDesktop([info("ready")]);
  const progress = [];
  const result = await ensureCodingAgentReady("http://127.0.0.1:9", "tok", (m) => progress.push(m), desktop.options);
  assert.equal(result.runtime.state, "ready");
  assert.equal(desktop.calls.length, 1);
  assert.equal(desktop.calls[0].headers["X-OpenMindAI-Token"], "tok");
  assert.equal(desktop.calls[0].headers["X-OpenMindAI-Client"], "vscode");
  assert.deepEqual(progress, []);
});

test("a stopped agent is started once and loading progress is reported with elapsed time", async () => {
  const desktop = scriptedDesktop(
    [info("loadingModel", { elapsedMs: 61_000 }), info("loadingModel", { elapsedMs: 155_000 }), info("ready")],
    info("starting", { elapsedMs: 0 })
  );
  desktop.calls.length = 0;
  const statuses = [info("stopped")];
  const progress = [];
  const options = {
    ...desktop.options,
    fetchJson: async (url, method, headers) => {
      if (statuses.length) return statuses.shift();
      return desktop.options.fetchJson(url, method, headers);
    }
  };
  const result = await ensureCodingAgentReady("http://127.0.0.1:9", "tok", (m) => progress.push(m), options);
  assert.equal(result.runtime.state, "ready");
  assert.equal(desktop.calls.filter((call) => call.method === "POST").length, 1);
  assert.equal(desktop.calls.find((call) => call.method === "POST").url, "http://127.0.0.1:9/openmindai/coding-agent/start");
  assert.deepEqual(progress, [
    "Starting OpenAgent Lite…",
    "OpenMindAI is loading OpenAgent Lite…",
    "OpenMindAI is loading OpenAgent Lite… 61s",
    "OpenMindAI is loading OpenAgent Lite… 155s"
  ]);
});

test("joining a load already in progress does not request another start", async () => {
  const desktop = scriptedDesktop([info("loadingModel", { elapsedMs: 5000 }), info("ready")]);
  await ensureCodingAgentReady("http://127.0.0.1:9", "tok", () => undefined, desktop.options);
  assert.equal(desktop.calls.filter((call) => call.method === "POST").length, 0);
});

test("disabled Coding Workspace gives a clear, actionable error", async () => {
  const desktop = scriptedDesktop([info("disabled")]);
  await assert.rejects(
    ensureCodingAgentReady("http://127.0.0.1:9", "tok", () => undefined, desktop.options),
    { message: DISABLED_MESSAGE }
  );
  assert.match(DISABLED_MESSAGE, /Agent Setup/);
});

test("a missing agent model is reported instead of falling back", async () => {
  const desktop = scriptedDesktop([
    { service: "openmindai-coding-agent", configured: false, message: "No OpenAgent coding model is installed.", runtime: { state: "notInstalled" } }
  ]);
  await assert.rejects(
    ensureCodingAgentReady("http://127.0.0.1:9", "tok", () => undefined, desktop.options),
    /No OpenAgent coding model is installed/
  );
});

test("a startup failure after starting is reported with the desktop's reason", async () => {
  const desktop = scriptedDesktop(
    [info("loadingModel", { elapsedMs: 1000 }), info("error", { error: "llama-server exited while loading the model" })],
    info("starting")
  );
  const statuses = [info("stopped")];
  const options = {
    ...desktop.options,
    fetchJson: async (url, method, headers) => (statuses.length ? statuses.shift() : desktop.options.fetchJson(url, method, headers))
  };
  await assert.rejects(
    ensureCodingAgentReady("http://127.0.0.1:9", "tok", () => undefined, options),
    /OpenAgent Lite could not be loaded: llama-server exited/
  );
});

test("a previous error triggers one retry start instead of failing immediately", async () => {
  const desktop = scriptedDesktop([info("ready")], info("starting"));
  const statuses = [info("error", { error: "old failure" })];
  const options = {
    ...desktop.options,
    fetchJson: async (url, method, headers) => (statuses.length ? statuses.shift() : desktop.options.fetchJson(url, method, headers))
  };
  const result = await ensureCodingAgentReady("http://127.0.0.1:9", "tok", () => undefined, options);
  assert.equal(result.runtime.state, "ready");
});

test("waiting gives up after the upper bound with a useful message", async () => {
  const desktop = scriptedDesktop([info("loadingModel", { elapsedMs: 1 })]);
  await assert.rejects(
    ensureCodingAgentReady("http://127.0.0.1:9", "tok", () => undefined, { ...desktop.options, timeoutMs: 10_000 }),
    /still loading after 10 seconds/
  );
});

test("loading message hides internals and shows whole seconds", () => {
  assert.equal(loadingMessage(info("loadingModel", { elapsedMs: 44_600 })), "OpenMindAI is loading OpenAgent Lite… 45s");
  assert.equal(loadingMessage(info("starting")), "OpenMindAI is loading OpenAgent Lite…");
});

test("an older desktop without runtime state is treated as ready", async () => {
  const desktop = scriptedDesktop([{ service: "openmindai-coding-agent", configured: true, model }]);
  await ensureCodingAgentReady("http://127.0.0.1:9", undefined, () => undefined, desktop.options);
  assert.equal(desktop.calls[0].headers["X-OpenMindAI-Token"], undefined);
});
