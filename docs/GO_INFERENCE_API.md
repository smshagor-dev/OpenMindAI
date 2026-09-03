# OpenMindAI Go Inference API

The Go inference API is a low-overhead gateway for external/web clients and backend integrations. It is **not** inserted between the desktop Tauri client and the native Rust/C++ inference engine, because doing so would add an unnecessary network hop to the fastest local path.

## Intended architecture

```text
Desktop React/Tauri
      |
      | direct in-process path
      v
Rust model router -> native CXX bridge -> llama.cpp -> GPU/CPU

Web / external clients
      |
      | HTTP + SSE
      v
Go inference API
      |
      | current compatibility upstream
      v
OpenAI-compatible llama-server

Native service mode (implemented, opt-in)
Local clients -> Go inference API -> private JSON pipes -> Rust/native worker -> C++ llama.cpp
```

The Go layer owns API-facing concerns that should not live in the inference kernel:

- persistent HTTP connection pooling;
- immediate SSE chunk flushing;
- request cancellation propagation when a client disconnects;
- bounded concurrent admission instead of unbounded inference pressure;
- short bounded queueing with `429` overload responses;
- health and readiness endpoints;
- request IDs;
- graceful process shutdown;
- a stable OpenAI-compatible `/v1/chat/completions` surface.

The Rust/C++ layer remains responsible for model ownership, chat templates, context/KV management, hardware backend selection, token generation, and native recovery.

## Run locally

```bash
cd services/inference-api
go run ./cmd/openmind-api
```

Defaults:

- API listen address: `127.0.0.1:11435`
- inference upstream: `http://127.0.0.1:8080`
- max active upstream requests: `4`
- overload queue timeout: `2s`
- maximum request body: `8 MiB`

The upstream is expected to expose an OpenAI-compatible chat endpoint at:

```text
POST /v1/chat/completions
```

For readiness, the gateway first probes `/health` and then `/v1/models` as a compatibility fallback.

## Environment variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `OPENMINDAI_API_ADDR` | `127.0.0.1:11435` | Go API bind address |
| `OPENMINDAI_INFERENCE_UPSTREAM` | `http://127.0.0.1:8080` | OpenAI-compatible inference upstream |
| `OPENMINDAI_API_MAX_INFLIGHT` | `4` | Maximum simultaneously admitted inference requests |
| `OPENMINDAI_API_QUEUE_TIMEOUT` | `2s` | Maximum time a saturated request can wait for a slot |
| `OPENMINDAI_API_MAX_BODY_BYTES` | `8388608` | Maximum request body size |
| `OPENMINDAI_API_READY_TIMEOUT` | `2s` | Upstream readiness probe timeout |
| `OPENMINDAI_API_READ_HEADER_TIMEOUT` | `10s` | HTTP header read timeout |
| `OPENMINDAI_API_IDLE_TIMEOUT` | `2m` | Idle keep-alive timeout |
| `OPENMINDAI_API_SHUTDOWN_TIMEOUT` | `10s` | Graceful shutdown deadline |

Invalid values fail fast during startup rather than silently falling back to unsafe values.

## Endpoints

### `GET /healthz`

Reports whether the Go API process itself is alive. It does not require the inference backend to be healthy.

### `GET /readyz`

Reports whether the configured inference upstream is responding successfully.

### `POST /v1/chat/completions`

Forwards an OpenAI-compatible JSON request. Streaming responses are passed through without application-level buffering and are flushed as chunks arrive.

When all inference slots are occupied beyond the configured queue timeout, the gateway returns HTTP `429` with `Retry-After: 1` instead of creating unbounded work and memory pressure.

## Performance notes

The Go gateway can reduce API-side overhead through connection reuse, lightweight goroutines, bounded scheduling, and direct streaming. It does **not** make llama.cpp token computation itself faster. Native token-generation performance comes from the C++/llama.cpp backend and GPU/CPU configuration.

For the desktop app, direct Rust/C++ inference remains the lowest-latency target. The Go API is valuable when there is a network/API boundary: web clients, local integrations, optional LAN service mode, automation, or a future browser UI.

## Native service mode

Set `OPENMINDAI_API_BACKEND=native` to replace the HTTP upstream with the
persistent Rust/CXX worker. The Go process owns private stdin/stdout pipes,
request IDs, process cancellation and restart. This mode accepts a documented
text-only chat subset, binds to loopback and serializes model requests.
See [native service setup](../services/native-worker/README.md) for registry,
build instructions, resource limits and real-model integration tests.
