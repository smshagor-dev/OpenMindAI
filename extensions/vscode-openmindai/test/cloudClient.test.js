const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const { test } = require("node:test");

const {
  CloudClient,
  DEFAULT_CLOUD_ENDPOINT,
  assertCloudEndpoint,
  fetchCloudModels,
  normalizeCloudEndpoint
} = require("../dist/cloudClient.js");

const root = path.join(__dirname, "..");
const messages = [
  { role: "system", content: "You are OpenMindAI." },
  { role: "user", content: "Say hello." }
];

/** Loopback server standing in for the OpenMindAI Cloud gateway. */
async function startServer(routes) {
  const requests = [];
  const server = http.createServer((req, res) => {
    let raw = "";
    req.on("data", (chunk) => (raw += chunk));
    req.on("end", () => {
      const body = raw ? JSON.parse(raw) : undefined;
      requests.push({ method: req.method, url: req.url, body, headers: req.headers });
      const handler = routes[`${req.method} ${req.url}`];
      if (!handler) {
        res.writeHead(404, { "Content-Type": "application/json" });
        res.end(JSON.stringify({ message: "not found" }));
        return;
      }
      handler(res, body);
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  return {
    endpoint: `http://127.0.0.1:${port}/api/v1/`,
    requests,
    close: () => new Promise((resolve) => server.close(resolve))
  };
}

function sendJson(res, status, value) {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(value));
}

function completion(content, finishReason = "stop") {
  return { choices: [{ index: 0, message: { role: "assistant", content }, finish_reason: finishReason }] };
}

function client(endpoint, extra = {}) {
  return new CloudClient({
    endpoint,
    apiKey: "sk-test-key",
    model: "openmindai-cloud",
    requestTimeoutMs: 5000,
    maxOutputTokens: 2048,
    ...extra
  });
}

test("normalizeCloudEndpoint adds a trailing slash and falls back to the default", () => {
  assert.equal(normalizeCloudEndpoint("https://host/api/v1"), "https://host/api/v1/");
  assert.equal(normalizeCloudEndpoint("https://host/api/v1///"), "https://host/api/v1/");
  assert.equal(normalizeCloudEndpoint(""), DEFAULT_CLOUD_ENDPOINT);
  assert.equal(normalizeCloudEndpoint("  "), DEFAULT_CLOUD_ENDPOINT);
  assert.equal(normalizeCloudEndpoint("auto"), DEFAULT_CLOUD_ENDPOINT);
  assert.equal(DEFAULT_CLOUD_ENDPOINT, "https://cloud.openmindai.org/api/v1/");
});

test("non-HTTPS endpoints are rejected unless the host is loopback", () => {
  assert.throws(() => assertCloudEndpoint("http://cloud.openmindai.org/api/v1/"), /must use HTTPS/);
  // Loopback HTTP is allowed (self-hosted gateway and tests).
  assert.ok(assertCloudEndpoint("http://127.0.0.1:8080/api/v1/"));
  assert.ok(assertCloudEndpoint("https://cloud.openmindai.org/api/v1/"));
  assert.throws(() => assertCloudEndpoint("not a url"), /valid URL/);
});

test("an empty API key is rejected at construction", () => {
  assert.throws(() => client("https://cloud.openmindai.org/api/v1/", { apiKey: "" }), /API key is not set/);
  assert.throws(() => client("https://cloud.openmindai.org/api/v1/", { apiKey: "   " }), /API key is not set/);
});

test("chat posts OpenAI-compatible fields with Bearer auth and returns content", async () => {
  const server = await startServer({
    "POST /api/v1/openmindai/chat/completions": (res) => sendJson(res, 200, completion("Hello there."))
  });
  try {
    const answer = await client(server.endpoint).chat(messages);
    assert.equal(answer, "Hello there.");
    assert.equal(server.requests.length, 1);
    const request = server.requests[0];
    assert.equal(request.url, "/api/v1/openmindai/chat/completions");
    assert.equal(request.headers.authorization, "Bearer sk-test-key");
    assert.equal(request.headers["x-openmindai-client"], "vscode");
    // The gateway rejects unknown fields; only standard OpenAI fields are sent.
    assert.deepEqual(Object.keys(request.body).sort(), ["max_tokens", "messages", "model", "stream"]);
    assert.equal(request.body.model, "openmindai-cloud");
    assert.equal(request.body.stream, false);
    assert.equal(request.body.max_tokens, 2048);
    assert.deepEqual(request.body.messages, messages);
  } finally {
    await server.close();
  }
});

test("server error bodies are surfaced, including the gateway top-level message field", async () => {
  const server = await startServer({
    "POST /api/v1/openmindai/chat/completions": (res) => sendJson(res, 401, { message: "Invalid API key." })
  });
  try {
    await assert.rejects(client(server.endpoint).chat(messages), { message: "Invalid API key." });
  } finally {
    await server.close();
  }
});

test("listModels keeps rows with a usable id and drops the rest", async () => {
  const server = await startServer({
    "GET /api/v1/openmindai/models": (res) =>
      sendJson(res, 200, {
        data: [
          { id: "openmindai-pro", name: "OpenMindAI Pro" },
          { id: "", name: "blank" },
          { name: "no id" },
          { id: "openmindai-lite" }
        ]
      })
  });
  try {
    const models = await fetchCloudModels(server.endpoint, "sk-test-key");
    assert.deepEqual(models, [
      { id: "openmindai-pro", name: "OpenMindAI Pro" },
      { id: "openmindai-lite", name: undefined }
    ]);
    assert.equal(server.requests[0].headers.authorization, "Bearer sk-test-key");
  } finally {
    await server.close();
  }
});

test("manifest exposes the cloud settings and the API key is never a plain setting", () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
  const properties = manifest.contributes.configuration.properties;
  assert.equal(properties["openmindai.backend"].default, "local");
  assert.deepEqual(properties["openmindai.backend"].enum, ["auto", "local", "cloud"]);
  assert.equal(properties["openmindai.cloudEndpoint"].default, DEFAULT_CLOUD_ENDPOINT);
  assert.ok("openmindai.cloudModel" in properties);
  // The API key must live in SecretStorage, never in settings.
  assert.ok(!("openmindai.cloudApiKey" in properties));
  const commands = manifest.contributes.commands.map((command) => command.command);
  for (const command of [
    "openmindai.selectBackend",
    "openmindai.setCloudApiKey",
    "openmindai.clearCloudApiKey",
    "openmindai.selectCloudModel"
  ]) {
    assert.ok(commands.includes(command), command);
  }
});
