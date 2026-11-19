const assert = require("node:assert/strict");
const http = require("node:http");
const { test } = require("node:test");

const {
  NATIVE_INTERFACE_ENDPOINT,
  findDesktopRuntimeEndpoint,
  isNativeInterfaceEndpoint,
  parseListeningPorts
} = require("../dist/runtimeDiscovery.js");

async function startServer(routes) {
  const server = http.createServer((req, res) => {
    const route = routes[req.url];
    if (!route) {
      res.writeHead(404);
      res.end("not found");
      return;
    }
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify(route));
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return { port: server.address().port, close: () => new Promise((resolve) => server.close(resolve)) };
}

async function closedPort() {
  const server = http.createServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

test("parses PowerShell listening-port output", () => {
  assert.deepEqual(parseListeningPorts("61292\r\n\r\n 5000 \r\n"), [61292, 5000]);
  assert.deepEqual(parseListeningPorts(""), []);
  assert.deepEqual(parseListeningPorts("abc\r\n-1\r\n0\r\n1.5\r\n8080"), [8080]);
});

test("runtime port auto-detection picks the first llama-server that answers like OpenMindAI", async () => {
  const dead = await closedPort();
  const notOpenMind = await startServer({ "/health": { status: "ok" } }); // no /v1/models
  const runtime = await startServer({ "/health": { status: "ok" }, "/v1/models": { object: "list", data: [] } });
  try {
    const endpoint = await findDesktopRuntimeEndpoint([dead, notOpenMind.port, runtime.port]);
    assert.equal(endpoint, `http://127.0.0.1:${runtime.port}`);
    assert.equal(await findDesktopRuntimeEndpoint([dead, notOpenMind.port]), undefined);
    assert.equal(await findDesktopRuntimeEndpoint([]), undefined);
  } finally {
    await notOpenMind.close();
    await runtime.close();
  }
});

test("recognizes the native interface fallback endpoint", () => {
  assert.equal(NATIVE_INTERFACE_ENDPOINT, "http://127.0.0.1:11435");
  assert.ok(isNativeInterfaceEndpoint("http://127.0.0.1:11435"));
  assert.ok(isNativeInterfaceEndpoint("http://localhost:11435/"));
  assert.ok(!isNativeInterfaceEndpoint("http://127.0.0.1:61292"));
  assert.ok(!isNativeInterfaceEndpoint("http://127.0.0.1:114350"));
});

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const {
  CODING_AGENT_SERVICE,
  codingAgentDescriptorPaths,
  describeTarget,
  findCodingAgentEndpoint,
  loadedModelName,
  requestJson
} = require("../dist/runtimeDiscovery.js");

const agentInfo = {
  service: CODING_AGENT_SERVICE,
  configured: true,
  model: {
    id: "gguf-b362ab78b3416c82",
    name: "OpenAgent Lite",
    family: "nemotron_h",
    repository: "nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF"
  },
  contextSize: 8192
};

function writeDescriptor(dir, descriptor) {
  const file = path.join(dir, "coding-agent-endpoint.json");
  fs.writeFileSync(file, JSON.stringify(descriptor));
  return file;
}

test("descriptor paths match where the desktop app writes them", () => {
  assert.deepEqual(
    codingAgentDescriptorPaths({ LOCALAPPDATA: "C:\\Users\\me\\AppData\\Local" }, "win32", "C:\\Users\\me"),
    [path.join("C:\\Users\\me\\AppData\\Local", "OpenMindAI", "coding-agent-endpoint.json")]
  );
  assert.deepEqual(codingAgentDescriptorPaths({}, "linux", "/home/me"), [
    path.join("/home/me", ".config", "OpenMindAI", "coding-agent-endpoint.json")
  ]);
  assert.equal(
    codingAgentDescriptorPaths({ OPENMINDAI_CODING_AGENT_DESCRIPTOR: "/tmp/x.json" }, "linux", "/h")[0],
    "/tmp/x.json"
  );
});

test("the Agent Setup coding agent is discovered from the desktop descriptor", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "openmindai-desc-"));
  const agent = await startServer({ "/openmindai/coding-agent": agentInfo });
  try {
    const descriptor = writeDescriptor(dir, {
      service: CODING_AGENT_SERVICE,
      endpoint: `http://127.0.0.1:${agent.port}/`
    });
    const target = await findCodingAgentEndpoint([descriptor]);
    assert.equal(target.kind, "coding-agent");
    assert.equal(target.endpoint, `http://127.0.0.1:${agent.port}`);
    assert.equal(target.agent.model.repository, "nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF");
    assert.equal(
      describeTarget(target),
      "OpenMindAI coding agent: OpenAgent Lite (nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF)"
    );
  } finally {
    await agent.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("stale, foreign, and non-loopback descriptors are ignored", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "openmindai-desc-"));
  const other = await startServer({ "/openmindai/coding-agent": { service: "something-else" } });
  try {
    const dead = await closedPort();
    for (const descriptor of [
      { service: CODING_AGENT_SERVICE, endpoint: `http://127.0.0.1:${dead}` },
      { service: CODING_AGENT_SERVICE, endpoint: `http://127.0.0.1:${other.port}` },
      { service: CODING_AGENT_SERVICE, endpoint: "http://192.168.1.10:8080" },
      { service: "not-openmindai", endpoint: `http://127.0.0.1:${other.port}` }
    ]) {
      assert.equal(
        await findCodingAgentEndpoint([writeDescriptor(dir, descriptor)]),
        undefined,
        JSON.stringify(descriptor)
      );
    }
    assert.equal(await findCodingAgentEndpoint([path.join(dir, "missing.json")]), undefined);
  } finally {
    await other.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a bare desktop llama-server is reported as bypassing Agent Setup", () => {
  assert.equal(
    loadedModelName({ data: [{ id: "G:\\portable ai\\models/llm/qwen/qwen3-4b/Qwen3-4B-Q4_K_M.gguf" }] }),
    "Qwen3-4B-Q4_K_M.gguf"
  );
  assert.equal(loadedModelName({ data: [] }), undefined);
  assert.equal(loadedModelName(undefined), undefined);
  const label = describeTarget({
    endpoint: "http://127.0.0.1:1",
    kind: "llama-server",
    loadedModel: "Qwen3-4B-Q4_K_M.gguf"
  });
  assert.match(label, /Qwen3-4B-Q4_K_M\.gguf loaded/);
  assert.match(label, /bypasses Agent Setup/);
});

test("discovery probes identify themselves to the desktop endpoint", async () => {
  let seen;
  const server = http.createServer((req, res) => {
    seen = req.headers["x-openmindai-client"];
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end("{}");
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    await requestJson(`http://127.0.0.1:${server.address().port}/health`, 1000);
    assert.equal(seen, "vscode");
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});

const { processAlive } = require("../dist/runtimeDiscovery.js");

/** Agent server that only answers with the right session token. */
async function startTokenAgent(token, instanceId) {
  const seen = [];
  const server = http.createServer((req, res) => {
    seen.push(req.headers["x-openmindai-token"]);
    if (req.headers["x-openmindai-token"] !== token) {
      res.writeHead(401, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: { message: "missing or invalid OpenMindAI session token" } }));
      return;
    }
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ ...agentInfo, instanceId }));
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return { port: server.address().port, seen, close: () => new Promise((resolve) => server.close(resolve)) };
}

test("discovery sends the descriptor token and checks the instance id", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "openmindai-desc-"));
  const agent = await startTokenAgent("session-token", "instance-1");
  try {
    const endpoint = `http://127.0.0.1:${agent.port}`;
    const good = writeDescriptor(dir, {
      service: CODING_AGENT_SERVICE,
      endpoint,
      pid: process.pid,
      instanceId: "instance-1",
      token: "session-token"
    });
    const target = await findCodingAgentEndpoint([good]);
    assert.equal(target.token, "session-token");
    assert.equal(agent.seen.at(-1), "session-token");

    // A reused port answered by a different OpenMindAI instance is not trusted.
    const otherInstance = writeDescriptor(dir, {
      service: CODING_AGENT_SERVICE,
      endpoint,
      pid: process.pid,
      instanceId: "instance-2",
      token: "session-token"
    });
    assert.equal(await findCodingAgentEndpoint([otherInstance]), undefined);

    // Wrong token: the endpoint refuses, so discovery skips it.
    const wrongToken = writeDescriptor(dir, {
      service: CODING_AGENT_SERVICE,
      endpoint,
      pid: process.pid,
      instanceId: "instance-1",
      token: "guess"
    });
    assert.equal(await findCodingAgentEndpoint([wrongToken]), undefined);
  } finally {
    await agent.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a descriptor whose process has exited is skipped without connecting", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "openmindai-desc-"));
  let probed = false;
  try {
    const descriptor = writeDescriptor(dir, {
      service: CODING_AGENT_SERVICE,
      endpoint: "http://127.0.0.1:1",
      pid: 999999,
      instanceId: "x",
      token: "t"
    });
    const result = await findCodingAgentEndpoint(
      [descriptor],
      async () => {
        probed = true;
        return {};
      },
      () => false
    );
    assert.equal(result, undefined);
    assert.equal(probed, false);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("only loopback hosts are accepted from the descriptor", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "openmindai-desc-"));
  let probed = 0;
  try {
    for (const endpoint of [
      "http://example.com:8080",
      "http://10.0.0.5:1234",
      "https://127.0.0.1:443",
      "http://127.0.0.1.evil.com:80",
      "http://127.0.0.1:80/../x",
      "http://user@127.0.0.1:80"
    ]) {
      const descriptor = writeDescriptor(dir, { service: CODING_AGENT_SERVICE, endpoint, token: "t" });
      const result = await findCodingAgentEndpoint(
        [descriptor],
        async () => {
          probed += 1;
          return { service: CODING_AGENT_SERVICE };
        },
        () => true
      );
      assert.equal(result, undefined, endpoint);
    }
    assert.equal(probed, 0);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("processAlive recognizes this process and rejects invalid ids", () => {
  assert.equal(processAlive(process.pid), true);
  assert.equal(processAlive(-1), false);
  assert.equal(processAlive("123"), false);
  assert.equal(processAlive(undefined), false);
});

const { hasLiveCodingAgentDescriptor } = require("../dist/runtimeDiscovery.js");

test("a live desktop descriptor blocks falling back to a bare llama-server", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "openmindai-desc-"));
  try {
    const live = writeDescriptor(dir, {
      service: CODING_AGENT_SERVICE,
      endpoint: "http://127.0.0.1:5000",
      pid: process.pid
    });
    assert.equal(hasLiveCodingAgentDescriptor([live]), true);
    assert.equal(hasLiveCodingAgentDescriptor([live], () => false), false);
    const remote = writeDescriptor(dir, {
      service: CODING_AGENT_SERVICE,
      endpoint: "http://10.0.0.1:5000",
      pid: process.pid
    });
    assert.equal(hasLiveCodingAgentDescriptor([remote]), false);
    assert.equal(hasLiveCodingAgentDescriptor([path.join(dir, "missing.json")]), false);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
