# Native inference service

This executable owns a persistent Rust/CXX/llama.cpp model worker. The Go API
launches it directly over bounded newline-delimited JSON pipes. No llama-server
or HTTP upstream is involved when `OPENMINDAI_API_BACKEND=native`.

Build against the pinned llama.cpp revision in `build.rs`, using the same shared
libraries as the desktop native bundle:

```sh
export LLAMA_CPP_DIR=/absolute/llama.cpp
export LLAMA_CPP_LIB_DIR=/absolute/llama.cpp/build/bin
export LD_LIBRARY_PATH="$LLAMA_CPP_LIB_DIR"
cargo build --locked --release --manifest-path services/native-worker/Cargo.toml
```

For Windows dynamic backends, also set `OPENMINDAI_NATIVE_DYNAMIC_BACKENDS=1`
and `LLAMA_CPP_BACKEND_LIB_DIR` to the directory containing `ggml.lib` and
`ggml-base.lib`. Put the matched runtime DLLs next to the worker executable;
use the validated native runtime packaging workflow. A Vulkan load failure can
retry CPU before any output. Never mix llama.cpp revisions or package modes.

Create an administrator-controlled registry outside the source checkout:

```json
{
  "openmindai-nano": {
    "path": "G:/portable ai/models/llm/qwen/qwen3-4b/Qwen3-4B-Q4_K_M.gguf",
    "gpu_layers": 99,
    "context_size": 8192
  }
}
```

Paths must be absolute, GGUF files must exist and be at most 8 GiB, and the
registry supports 1–32 model IDs. Clients can select IDs, never filesystem paths.
Choose GPU layers and context limits for the actual device.

```sh
export OPENMINDAI_API_BACKEND=native
export OPENMINDAI_NATIVE_WORKER=/absolute/openmind-native-worker
export OPENMINDAI_NATIVE_MODELS=/absolute/models.json
export OPENMINDAI_API_GENERATION_TIMEOUT=120s
cd services/inference-api
go run ./cmd/openmind-api
```

The native API binds only to a loopback IP, defaults to port 11435 and admits one
request at a time. Its existing queue timeout returns 429 when busy. It accepts
`application/json` requests to `/v1/chat/completions`, with `model`, text
`messages`, `stream`, `temperature`, `top_p` and `max_tokens`. Unsupported fields
and multimodal/tool messages are rejected. The HTTP proxy backend retains its
existing compatibility behavior. Native readiness confirms registry validation
and the version-1 worker handshake; it does not load or benchmark every model.

Native requests are below 1 MiB, contain at most 256 messages, use at most 8192
output tokens and a context cap of 32768. Per-request deadlines default to two
minutes and cannot exceed one hour. The bridge rejects over-limit prompts
before allocating a context, estimates dense F16 KV admission memory, and frees
old context buffers before resizing. This estimate is not a hard RSS/VRAM cap;
weights, compute buffers and driver allocations also consume memory.

On disconnect, timeout, a malformed response or worker failure, Go kills and
reaps the worker. The next request starts a fresh process; a partially streamed
answer is never retried. Stream errors emit an error event without a success
`[DONE]` frame. Slow stream writes have a five-second deadline. Completion
`finish_reason` is currently `stop`; the bridge does not expose exact token
usage or distinguish the token limit from EOS.

Desktop inference remains in-process. An unresponsive native call returns a
bounded timeout and quarantines that supervisor; application restart releases
the runtime. In-process GPU driver recovery cannot safely kill a C++ thread.
Neither this change nor CPU CI verifies RX580 hardware or enables Vulkan as the
production release default.

## Validation

`go test -race ./...` in `services/inference-api` tests process reuse, cancellation,
crashes, invalid IDs, oversized frames, overload and HTTP/SSE error handling.
Set `OPENMINDAI_TEST_NATIVE_WORKER` and `OPENMINDAI_TEST_MODEL` to enable real
HTTP → Go → Rust/CXX generation, cancellation and subsequent recovery.
The Native CXX workflow builds the worker and executes this integration with a
checksum-pinned tiny GGUF. The native smoke runner also checks context rejection,
generation deadlines and recovery. The tiny fixture tests mechanics, not answer
quality or device performance.

## Optional personal adapters

A registry entry can set `personalization` to an absolute per-profile activation
JSON path produced by the [personalization CLI](../../experiments/python/README.md).
The worker reads the pointer on each request, checks profile/model evaluation
metadata, and verifies exact base/adapter hashes when loading. Changes reload the
resident model. A null candidate selects the base model. Invalid activation
blocks the request. The CLI supports real CPU LoRA training, native activation
probes and rollback; Python remains optional for inference.
