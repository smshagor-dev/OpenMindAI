const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const { test } = require("node:test");

const {
  DEFAULT_MAX_OUTPUT_TOKENS,
  EMPTY_RESPONSE_MESSAGE,
  MAX_MAX_OUTPUT_TOKENS,
  MIN_MAX_OUTPUT_TOKENS,
  OpenMindClient,
  REASONING_ONLY_MESSAGE,
  TOKEN_LIMIT_MESSAGE,
  CONTEXT_SAFETY_MARGIN,
  buildChatRequestBody,
  estimatePromptTokens,
  extractAssistantAnswer,
  fitOutputBudget,
  resolveMaxOutputTokens
} = require("../dist/openmindClient.js");

const root = path.join(__dirname, "..");
const messages = [
  { role: "system", content: "You are OpenMindAI inside VS Code." },
  { role: "user", content: "Explain what a Rust enum is in two sentences." }
];

/**
 * Starts a loopback chat server; handler(body, res) answers chat POSTs and `gets` maps GET
 * paths (such as llama-server /props) to JSON. `requests` records chat POSTs only.
 */
async function startServer(handler, gets = {}) {
  const requests = [];
  const getRequests = [];
  const server = http.createServer((req, res) => {
    let raw = "";
    req.on("data", (chunk) => (raw += chunk));
    req.on("end", () => {
      if (req.method === "GET") {
        getRequests.push(req.url);
        if (gets[req.url]) sendJson(res, 200, gets[req.url]);
        else sendJson(res, 404, { error: { message: "not found" } });
        return;
      }
      const body = raw ? JSON.parse(raw) : undefined;
      requests.push({ url: req.url, body, headers: req.headers });
      handler(body, res, req);
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  return {
    endpoint: `http://127.0.0.1:${port}`,
    port,
    requests,
    getRequests,
    close: () => new Promise((resolve) => server.close(resolve))
  };
}

function sendJson(res, status, value) {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(value));
}

function completion(message, finishReason = "stop") {
  return { object: "chat.completion", choices: [{ index: 0, message, finish_reason: finishReason }] };
}

function client(endpoint, extra = {}) {
  return new OpenMindClient({
    endpoint,
    model: "openmind-local",
    requestTimeoutMs: 5000,
    maxOutputTokens: DEFAULT_MAX_OUTPUT_TOKENS,
    ...extra
  });
}

test("normal response returns content and sends enable_thinking: false", async () => {
  const server = await startServer((_body, res) =>
    sendJson(res, 200, completion({ role: "assistant", content: "An enum is a sum type." }))
  );
  try {
    const answer = await client(server.endpoint).chat(messages);
    assert.equal(answer, "An enum is a sum type.");
    assert.equal(server.requests.length, 1);
    const body = server.requests[0].body;
    assert.equal(server.requests[0].url, "/v1/chat/completions");
    assert.deepEqual(body.chat_template_kwargs, { enable_thinking: false });
    assert.equal(body.max_tokens, DEFAULT_MAX_OUTPUT_TOKENS);
    assert.equal(body.stream, false);
    assert.equal(body.model, "openmind-local");
    assert.deepEqual(body.messages, messages);
  } finally {
    await server.close();
  }
});

test("empty content + reasoning_content + length gives the token-limit error without leaking reasoning", async () => {
  const reasoning = "Okay, the user is asking about secret internal reasoning";
  const server = await startServer((_body, res) =>
    sendJson(
      res,
      200,
      completion({ role: "assistant", content: "", reasoning_content: reasoning }, "length")
    )
  );
  try {
    await assert.rejects(client(server.endpoint).chat(messages), (error) => {
      assert.equal(error.message, TOKEN_LIMIT_MESSAGE);
      assert.ok(!error.message.includes("secret internal reasoning"));
      assert.ok(error.message.includes("openmindai.maxOutputTokens"));
      return true;
    });
  } finally {
    await server.close();
  }
});

test("reasoning_content is never treated as the answer", () => {
  assert.throws(
    () =>
      extractAssistantAnswer(
        completion({ role: "assistant", content: "", reasoning_content: "thinking..." }, "stop")
      ),
    { message: REASONING_ONLY_MESSAGE }
  );
  assert.throws(
    () => extractAssistantAnswer(completion({ role: "assistant", reasoning_content: "thinking..." })),
    { message: REASONING_ONLY_MESSAGE }
  );
});

test("empty response without useful content gives a specific error", () => {
  assert.throws(() => extractAssistantAnswer(completion({ role: "assistant", content: "" })), {
    message: EMPTY_RESPONSE_MESSAGE
  });
  assert.throws(() => extractAssistantAnswer(completion({ role: "assistant", content: "   \n" })), {
    message: EMPTY_RESPONSE_MESSAGE
  });
  assert.throws(() => extractAssistantAnswer({}), { message: EMPTY_RESPONSE_MESSAGE });
  assert.throws(() => extractAssistantAnswer({ choices: [] }), { message: EMPTY_RESPONSE_MESSAGE });
  assert.throws(() => extractAssistantAnswer(undefined), { message: EMPTY_RESPONSE_MESSAGE });
});

test("a valid answer that stops exactly at the token limit is still returned", () => {
  const answer = extractAssistantAnswer(
    completion({ role: "assistant", content: "Partial but real answer" }, "length")
  );
  assert.equal(answer, "Partial but real answer");
});

test("inline <think> blocks are stripped and content parts are supported", () => {
  assert.equal(
    extractAssistantAnswer(completion({ role: "assistant", content: "<think>\nplan\n</think>\n\nFinal answer." })),
    "Final answer."
  );
  assert.throws(
    () => extractAssistantAnswer(completion({ role: "assistant", content: "<think>unfinished plan" }, "length")),
    { message: TOKEN_LIMIT_MESSAGE }
  );
  assert.equal(
    extractAssistantAnswer(
      completion({ role: "assistant", content: [{ type: "text", text: "Hello " }, { type: "text", text: "world" }] })
    ),
    "Hello world"
  );
  assert.equal(extractAssistantAnswer({ choices: [{ text: "legacy completion", finish_reason: "stop" }] }), "legacy completion");
});

test("server error bodies are surfaced", async () => {
  const server = await startServer((_body, res) =>
    sendJson(res, 503, { error: { message: "model is loading" } })
  );
  try {
    await assert.rejects(client(server.endpoint).chat(messages), { message: "model is loading" });
  } finally {
    await server.close();
  }
  assert.throws(() => extractAssistantAnswer({ error: { message: "bad model" } }), { message: "bad model" });
});

test("custom openmindai.maxOutputTokens is sent as max_tokens", async () => {
  const server = await startServer((_body, res) =>
    sendJson(res, 200, completion({ role: "assistant", content: "ok" }))
  );
  try {
    await client(server.endpoint, { maxOutputTokens: 4096 }).chat(messages);
    await client(server.endpoint, { maxOutputTokens: 300 }).chat(messages);
    assert.equal(server.requests[0].body.max_tokens, 4096);
    assert.equal(server.requests[1].body.max_tokens, 300);
  } finally {
    await server.close();
  }
});

test("invalid token values are validated", () => {
  for (const invalid of [-5, 0, NaN, Infinity, "abc", undefined, null, {}]) {
    assert.equal(resolveMaxOutputTokens(invalid), DEFAULT_MAX_OUTPUT_TOKENS, String(invalid));
  }
  assert.equal(resolveMaxOutputTokens(3), MIN_MAX_OUTPUT_TOKENS);
  assert.equal(resolveMaxOutputTokens(99999), MAX_MAX_OUTPUT_TOKENS);
  assert.equal(resolveMaxOutputTokens(300.7), 300);
  assert.equal(resolveMaxOutputTokens("1024"), 1024);
  assert.equal(buildChatRequestBody("m", messages, -1, "llama-server").max_tokens, DEFAULT_MAX_OUTPUT_TOKENS);
});

test("default token value is 2048 in code and in the contributed setting", () => {
  assert.equal(DEFAULT_MAX_OUTPUT_TOKENS, 2048);
  const manifest = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
  const setting = manifest.contributes.configuration.properties["openmindai.maxOutputTokens"];
  assert.equal(setting.default, 2048);
  assert.equal(setting.minimum, MIN_MAX_OUTPUT_TOKENS);
  assert.equal(setting.maximum, MAX_MAX_OUTPUT_TOKENS);
  const extensionSource = fs.readFileSync(path.join(root, "src", "extension.ts"), "utf8");
  assert.match(extensionSource, /get<number>\("maxOutputTokens", DEFAULT_MAX_OUTPUT_TOKENS\)/);
});

test("native-gateway profile omits chat_template_kwargs", () => {
  const body = buildChatRequestBody("openmind-local", messages, 512, "native-gateway");
  assert.equal(body.chat_template_kwargs, undefined);
  assert.deepEqual(Object.keys(body).sort(), ["max_tokens", "messages", "model", "stream"]);
});

test("a strict native gateway at a custom endpoint is retried once without extensions", async () => {
  // Mirrors services/inference-api decodeNative(), which uses DisallowUnknownFields().
  const allowed = new Set(["model", "messages", "stream", "temperature", "top_p", "max_tokens"]);
  const server = await startServer((body, res) => {
    if (Object.keys(body).some((key) => !allowed.has(key))) {
      sendJson(res, 400, { error: { message: "unsupported or invalid native chat fields", type: "gateway_error" } });
      return;
    }
    sendJson(res, 200, completion({ role: "assistant", content: "native ok" }));
  });
  try {
    assert.equal(await client(server.endpoint).chat(messages), "native ok");
    assert.equal(server.requests.length, 2);
    assert.ok(server.requests[0].body.chat_template_kwargs);
    assert.equal(server.requests[1].body.chat_template_kwargs, undefined);

    // Remembered for later requests: no extra rejected round-trip.
    assert.equal(await client(server.endpoint).chat(messages), "native ok");
    assert.equal(server.requests.length, 3);
    assert.equal(server.requests[2].body.chat_template_kwargs, undefined);
  } finally {
    await server.close();
  }
});

test("coding-agent profile sends only standard fields and reports the Agent Setup model", async () => {
  const route = {
    route: "coding-agent",
    model: { id: "gguf-b362", name: "OpenAgent Lite", repository: "nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF" },
    contextSize: 4096,
    promptTokens: 300,
    requestedMaxTokens: 3000,
    maxTokens: 2048
  };
  const server = await startServer((_body, res) =>
    sendJson(res, 200, {
      ...completion({ role: "assistant", content: "ok" }),
      usage: { prompt_tokens: 300, completion_tokens: 2, total_tokens: 302 },
      openmindai: route
    })
  );
  try {
    const result = await client(server.endpoint, { profile: "coding-agent", maxOutputTokens: 3000 }).chatDetailed(messages);
    assert.equal(result.content, "ok");
    assert.equal(result.finishReason, "stop");
    assert.deepEqual(result.usage, { prompt_tokens: 300, completion_tokens: 2, total_tokens: 302 });
    assert.deepEqual(result.route, route);
    // The desktop reports the budget it actually used after fitting the context window.
    assert.equal(result.maxTokens, 2048);
    const body = server.requests[0].body;
    // The desktop coding agent owns model-specific template options and context budgeting.
    assert.deepEqual(Object.keys(body).sort(), ["max_tokens", "messages", "model", "stream"]);
    assert.equal(body.max_tokens, 3000);
    assert.equal(server.requests[0].headers["x-openmindai-client"], "vscode");
    assert.deepEqual(server.getRequests, []);
  } finally {
    await server.close();
  }
});

test("llama-server output budget uses the slot context reported by /props", async () => {
  const server = await startServer(
    (_body, res) => sendJson(res, 200, completion({ role: "assistant", content: "ok" })),
    { "/props": { default_generation_settings: { n_ctx: 1024 } } }
  );
  try {
    const result = await client(server.endpoint, { maxOutputTokens: 2048 }).chatDetailed(messages);
    const expected = 1024 - estimatePromptTokens(messages) - CONTEXT_SAFETY_MARGIN;
    assert.equal(server.requests[0].body.max_tokens, expected);
    assert.equal(result.maxTokens, expected);
    assert.deepEqual(server.getRequests, ["/props"]);
  } finally {
    await server.close();
  }
});

test("a prompt that cannot fit the context window fails before sending", async () => {
  const server = await startServer(
    (_body, res) => sendJson(res, 200, completion({ role: "assistant", content: "ok" })),
    { "/props": { default_generation_settings: { n_ctx: 512 } } }
  );
  try {
    const huge = [{ role: "user", content: "x".repeat(3000) }];
    await assert.rejects(client(server.endpoint).chat(huge), /does not fit the model context window \(512 tokens\)/);
    assert.equal(server.requests.length, 0);
  } finally {
    await server.close();
  }
});

test("native-gateway budgets against the configured native context without extra fields", async () => {
  const server = await startServer((_body, res) => sendJson(res, 200, completion({ role: "assistant", content: "ok" })));
  try {
    await client(server.endpoint, { profile: "native-gateway", contextSize: 1024, maxOutputTokens: 512 }).chat(messages);
    const body = server.requests[0].body;
    assert.deepEqual(Object.keys(body).sort(), ["max_tokens", "messages", "model", "stream"]);
    assert.equal(body.max_tokens, 512);
    assert.deepEqual(server.getRequests, []);
  } finally {
    await server.close();
  }
});

test("fitOutputBudget keeps the requested budget when it fits and shrinks otherwise", () => {
  assert.equal(fitOutputBudget(2048, 8192, 1000), 2048);
  assert.equal(fitOutputBudget(2048, 2048, 1000), 2048 - 1000 - CONTEXT_SAFETY_MARGIN);
  assert.equal(fitOutputBudget(2048, 1024, 1000), undefined);
  assert.equal(fitOutputBudget(-1, 8192, 10), DEFAULT_MAX_OUTPUT_TOKENS);
});
