# Native llama.cpp inference bridge

This branch introduces an **optional** native inference path. The existing Tauri/Rust runtime remains the default until the new path is benchmarked and deliberately wired into chat routing.

## Why this boundary

`cxx` is used instead of raw `extern "C"` pointers. Rust owns the streaming callback (`TokenSink`), C++ receives it as an opaque CXX type, and strings cross the boundary as borrowed `rust::Str` / `&str`. The C++ engine is owned by Rust through `cxx::UniquePtr`, so model/context destruction follows RAII and no manual raw-pointer lifetime is exposed to application code.

The bridge currently passes:

- user prompt
- system prompt
- `temperature`
- `top_p`
- `max_tokens`
- context/batch/thread sizing
- GPU layer count at model load

Tokens are returned synchronously, token-by-token, through `TokenSink::on_token`. Returning `false` from the Rust callback cancels generation without exposing a C function pointer or unsafe Rust callback trampoline.

## Current layout

```text
OpenMindAI/
├─ src-tauri/
│  ├─ Cargo.toml
│  ├─ build.rs
│  ├─ native/
│  │  ├─ inference.h
│  │  └─ inference.cpp
│  └─ src/
│     ├─ native_bridge.rs
│     └─ ...existing Tauri/Rust core
├─ docs/
│  └─ NATIVE_INFERENCE_BRIDGE.md
└─ ...existing React/Tauri UI
```

Recommended workspace evolution when the Node/Next.js service is added:

```text
OpenMindAI/
├─ crates/
│  ├─ openmind-core/            # Rust orchestration, model/session API
│  └─ openmind-node/            # napi-rs cdylib; depends on openmind-core
├─ native/
│  └─ llama/
│     ├─ inference.h
│     └─ inference.cpp
├─ apps/
│  ├─ desktop/                  # Tauri + React
│  └─ web/
│     └─ server/                # Node/TypeScript WebSocket backend
└─ vendor/llama.cpp/            # pinned submodule or CI checkout
```

Do not make Tauri itself the Node ABI. Extract the reusable orchestration into `openmind-core`, then let both the Tauri application and `openmind-node` depend on that crate.

## Build prerequisites

The native bridge is feature-gated and does not affect normal builds:

```bash
cargo build --manifest-path src-tauri/Cargo.toml
```

To enable it, build llama.cpp separately, then provide its source/include root and library output directory:

```bash
export LLAMA_CPP_DIR=/absolute/path/to/llama.cpp
export LLAMA_CPP_LIB_DIR=/absolute/path/to/llama.cpp/build/bin
cargo build \
  --manifest-path src-tauri/Cargo.toml \
  --features native-cxx-llama
```

On Windows PowerShell:

```powershell
$env:LLAMA_CPP_DIR = "C:\src\llama.cpp"
$env:LLAMA_CPP_LIB_DIR = "C:\src\llama.cpp\build\bin\Release"
cargo build --manifest-path src-tauri\Cargo.toml --features native-cxx-llama
```

The default linker mode is dynamic (`llama.dll`, `libllama.so`, or `libllama.dylib`). Override with:

```text
OPENMINDAI_LLAMA_LINK_KIND=static
```

Static llama.cpp builds normally need additional ggml libraries. Supply them explicitly, for example:

```text
OPENMINDAI_LLAMA_EXTRA_LIBS=ggml;ggml-base;ggml-cpu
```

For a static CUDA build this commonly also includes `ggml-cuda`, plus CUDA runtime libraries required by the llama.cpp build.

## CUDA and CPU SIMD

The wrapper is compiled at `-O3` (or `/O2` on MSVC). Non-portable local builds also request the host CPU ISA (`-march=native`, `-mcpu=native`, or `/arch:AVX2`). Set:

```text
OPENMINDAI_PORTABLE_BUILD=1
```

for release artifacts that must run on older CPUs.

Important: AVX2/AVX512 and CUDA kernels live in **llama.cpp/ggml**, not in the tiny OpenMindAI wrapper. `build.rs` cannot retrofit CUDA into an already-built llama library. Build llama.cpp itself with the intended backend, for example:

```bash
cmake -S "$LLAMA_CPP_DIR" -B "$LLAMA_CPP_DIR/build" \
  -DGGML_NATIVE=ON \
  -DGGML_CUDA=ON \
  -DBUILD_SHARED_LIBS=ON \
  -DLLAMA_BUILD_TESTS=OFF \
  -DLLAMA_BUILD_EXAMPLES=OFF
cmake --build "$LLAMA_CPP_DIR/build" --config Release -j
```

For CPU-only portable builds, turn CUDA off and choose the SIMD baseline in the llama.cpp build rather than relying on the wrapper compiler flags.

`OPENMINDAI_LLAMA_CUDA=1` tells the Rust build that the linked llama backend is CUDA-enabled and adds CUDA SDK library search paths when `CUDA_PATH`/`CUDA_HOME` is available. The actual backend selection remains llama.cpp's responsibility.

## Dynamic KV cache policy

`InferenceEngine` loads the GGUF model once. The llama context/KV cache is created on demand for each required context size and reused across generations when capacity is suitable.

The target context is based on:

```text
max(configured_n_ctx, prompt_tokens + max_tokens + safety_margin)
```

and rounded to a small allocation block. The context grows when needed and shrinks when it is more than twice the current requirement. Before a new generation, `llama_memory_clear` resets sequence memory while keeping the allocated context reusable.

This avoids reserving a huge long-context KV cache for every short request while also avoiding a context rebuild on every message.

For very large project files, the application layer should still use retrieval/chunking and a token budget. Dynamic KV allocation is not a substitute for bounded context construction.

## Rust usage

```rust
#[cfg(feature = "native-cxx-llama")]
use open_mind_ai_lib::native_bridge::{GenerationConfig, NativeInferenceEngine};

let mut engine = NativeInferenceEngine::load("models/qwen.gguf", -1)?;
let config = GenerationConfig {
    temperature: 0.7,
    top_p: 0.9,
    max_tokens: 512,
    n_ctx: 8_192,
    n_batch: 512,
    n_threads: 8,
    n_gpu_layers: -1,
};

engine.generate(
    "Explain this project",
    "You are a concise coding assistant.",
    config,
    |token| {
        // Forward immediately to Tauri event, channel, or socket producer.
        print!("{token}");
        true // false cancels generation
    },
)?;
```

For asynchronous application code, run the blocking llama generation on a dedicated worker thread / `spawn_blocking`; never block the Tokio async executor with token generation.

## Node.js / TypeScript integration

The cleanest boundary is **napi-rs**, not a second C ABI layer and not Node calling the C++ wrapper directly.

Recommended flow:

```text
Next.js client
    │ WebSocket / SSE
    ▼
Node.js TypeScript server
    │ napi-rs ThreadsafeFunction / AsyncTask
    ▼
openmind-node (Rust cdylib)
    │ normal Rust API
    ▼
openmind-core
    │ cxx
    ▼
inference.cpp -> llama.cpp -> CUDA / CPU
```

`openmind-node` should expose a `generate()` method that accepts JS options and a JS token callback. Inside the Rust N-API crate:

1. Convert the JS callback to a `ThreadsafeFunction<String>` (or napi-rs equivalent current API).
2. Start blocking inference on a worker thread.
3. Pass a Rust `TokenSink` closure into `NativeInferenceEngine::generate`.
4. For every token, enqueue the token on the N-API threadsafe callback.
5. The TypeScript layer forwards that token immediately over the request's WebSocket/SSE channel.
6. An `AbortController`/request ID should flip an atomic cancellation flag; the `TokenSink` returns `false` on the next token.

Do not invoke V8/Node APIs from the llama inference thread directly. The threadsafe N-API callback is the required hop back to the Node event loop.

A minimal TypeScript boundary should look like:

```ts
native.generate(
  {
    modelPath,
    prompt,
    systemPrompt,
    temperature: 0.7,
    topP: 0.9,
    maxTokens: 512,
  },
  (token: string) => socket.send(JSON.stringify({ type: "token", token })),
);
```

Keep WebSocket connection state in TypeScript. Keep model ownership, context sizing, cancellation, and inference state in Rust/C++.

## System prompt formatting

The initial wrapper deliberately keeps prompt composition simple and model-independent. Before this path becomes OpenMindAI's default chat runtime, route prompt construction through llama.cpp's model chat-template API (or the existing OpenMindAI model metadata) so Qwen, Llama, Gemma, Mistral, and other GGUF chat templates receive their native control tokens.

Do not hard-code one ChatML format globally.

## Production checklist before routing real chat traffic

- Pin a tested llama.cpp commit instead of building arbitrary `master`.
- Add a CI job with a small test GGUF and the `native-cxx-llama` feature.
- Add CUDA and CPU-only smoke matrices.
- Add cancellation, long-context, invalid-model, and OOM tests.
- Add model-template-aware prompt formatting.
- Add bounded concurrent-generation scheduling; one mutable context must not be used concurrently.
- Decide shared-library packaging/rpath/DLL deployment for each OS.
- Benchmark the native path against the current llama runtime before replacing existing routing.
