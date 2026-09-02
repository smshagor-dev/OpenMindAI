# OpenMindAI Native llama.cpp Bridge

This branch introduces a standalone native inference layer without replacing the current desktop llama-server/runtime path. The native path can be integrated behind a feature flag after it is validated on the release hardware matrix.

## Architecture

```text
OpenMindAI/
├─ native-core/
│  ├─ Cargo.toml                 # Rust core crate + cxx dependency
│  ├─ build.rs                   # CMake llama.cpp + cxx C++ wrapper build
│  ├─ include/
│  │  └─ inference.h             # Small C++ API visible through cxx
│  ├─ cpp/
│  │  └─ inference.cpp           # GGUF load, sampling, KV/context management
│  ├─ src/
│  │  ├─ lib.rs                  # Safe public Rust API
│  │  └─ bridge.rs               # #[cxx::bridge] + streaming token sink
│  └─ vendor/
│     └─ llama.cpp/              # Optional pinned llama.cpp checkout
├─ native-node/
│  ├─ Cargo.toml                 # napi-rs cdylib
│  ├─ build.rs
│  ├─ package.json
│  ├─ src/lib.rs                 # Dedicated engine worker + ThreadsafeFunction
│  └─ examples/ws-server.ts      # WebSocket token relay example
├─ src-tauri/                    # Existing desktop application, unchanged by default
└─ src/                          # Existing React/Tauri UI
```

## Why `cxx` instead of a raw C ABI

The bridge keeps ownership explicit: Rust owns a `UniquePtr<InferenceEngine>`, C++ owns llama.cpp model/context resources behind a PIMPL, and generation only borrows the engine and Rust token sink. C++ exceptions are translated into Rust `Result` by `cxx`. There are no manually transmuted pointers, caller-owned raw buffers, or lifetime-free callback userdata pointers.

Generated token pieces cross the FFI boundary as `&[u8]`, not `&str`. llama tokenization can split a multi-byte UTF-8 code point across pieces; Rust buffers incomplete byte prefixes and only forwards valid UTF-8 chunks.

## Request path

```text
TypeScript/Node
  -> napi-rs NativeLlama worker command
  -> openmind-native-core (Rust)
  -> cxx typed bridge
  -> inference.cpp
  -> llama_decode / sampler chain
  -> token bytes
  -> Rust UTF-8 assembler
  -> bounded channel
  -> napi-rs ThreadsafeFunction
  -> WebSocket
  -> Next.js/React client
```

The N-API object never moves the C++ engine pointer onto the JavaScript event-loop thread. A dedicated Rust worker creates and owns the engine. A bounded token channel and blocking ThreadsafeFunction calls provide backpressure so a slow WebSocket client cannot cause an unbounded token queue.

## llama.cpp source pin

The wrapper was authored against llama.cpp master commit:

```text
7798007a29a90e3053e799394da48cf53a2f8e0f
```

For reproducible builds, use a pinned checkout rather than tracking master:

```bash
git clone https://github.com/ggml-org/llama.cpp native-core/vendor/llama.cpp
cd native-core/vendor/llama.cpp
git checkout 7798007a29a90e3053e799394da48cf53a2f8e0f
```

Alternatively point at an external checkout:

```bash
export OPENMINDAI_LLAMA_DIR=/absolute/path/to/llama.cpp
```

PowerShell:

```powershell
$env:OPENMINDAI_LLAMA_DIR = "G:\deps\llama.cpp"
```

## CPU and CUDA builds

`native-core/build.rs` configures llama.cpp through CMake and compiles the cxx wrapper as C++17.

Default x86-64 release behavior uses AVX2/FMA as a portable high-performance baseline. For a machine-specific build:

```bash
OPENMINDAI_NATIVE_TUNE=1 cargo build --manifest-path native-core/Cargo.toml --release
```

`OPENMINDAI_NATIVE_TUNE=1` enables `GGML_NATIVE` and `-march=native` where supported. Do not ship that binary to unknown CPUs because compiler-generated instructions can exceed the target machine's ISA. AVX-512 should therefore be produced as a separate hardware-targeted artifact, not as the universal Windows binary.

CUDA is enabled automatically when `nvcc` is available. It can be forced explicitly:

```powershell
$env:OPENMINDAI_LLAMA_CUDA = "1"
cargo build --manifest-path native-core/Cargo.toml --release
```

Force CPU-only:

```powershell
$env:OPENMINDAI_LLAMA_CUDA = "0"
cargo build --manifest-path native-core/Cargo.toml --release
```

At runtime, `EngineConfig.gpu_layers = -1` requests maximum supported offload, while `0` forces CPU inference. The actual CUDA acceleration comes from llama.cpp/ggml compiled with `GGML_CUDA=ON`; the wrapper itself stays backend-agnostic.

## Dynamic context / KV cache behavior

The model is loaded once. The llama context/KV cache is allocated lazily on first generation and grows geometrically (512, 1024, 2048, ...) until it can hold `prompt_tokens + max_tokens`, capped by the model's training context. A larger context is reused for later smaller requests rather than reallocated. KV state is cleared between independent generations.

Prompt prefill is decoded in bounded chunks using the context's `n_batch`, avoiding the need to allocate one giant batch for long project files.

If a request exceeds the model's supported context window, generation fails explicitly instead of silently truncating project data. A later retrieval/chunking layer should decide which project context is relevant; the inference wrapper should not make hidden semantic truncation decisions.

## Rust API

```rust
use std::sync::mpsc;
use openmind_native_core::{EngineConfig, GenerateConfig, NativeLlamaEngine};

let mut engine = NativeLlamaEngine::load(
    "models/qwen.gguf",
    EngineConfig {
        base_context_tokens: 8192,
        gpu_layers: -1,
    },
)?;

let (tx, rx) = mpsc::sync_channel(128);
std::thread::spawn(move || {
    while let Ok(token) = rx.recv() {
        // Forward token to Tauri event, WebSocket, SSE, etc.
        println!("{token}");
    }
});

engine.generate_to_sender(
    "Explain this project",
    "You are OpenMindAI.",
    GenerateConfig {
        temperature: 0.7,
        top_p: 0.9,
        max_tokens: 1024,
    },
    tx,
)?;
```

The C++ call is synchronous by design. Run it on a dedicated worker or blocking thread, never on an async runtime/event-loop executor thread.

## Node.js / Next.js integration

Use `napi-rs`, not ad-hoc Node FFI into a C++ DLL. The N-API layer can reuse Rust validation, ownership, error handling, and streaming code, while `ThreadsafeFunction` safely returns token events from the native worker to JavaScript.

Build the addon:

```bash
cd native-node
npm install
npm run build
```

The addon exposes:

```ts
const engine = new NativeLlama(modelPath, {
  baseContextTokens: 8192,
  gpuLayers: -1,
});

engine.generate(
  prompt,
  systemPrompt,
  { temperature: 0.7, topP: 0.9, maxTokens: 1024 },
  (kind, data) => {
    // kind: token | done | error
  },
);
```

For self-hosted Next.js, keep the native addon in the Node runtime only (`server-only` module) and relay events through a WebSocket gateway. Do not import the native module into Client Components or Edge Runtime code. For serverless/edge deployments, run the native inference process as a separate long-lived service because GGUF model residency and GPU contexts require persistent process state.

## Security boundaries

- Validate generation parameters in Rust and again in C++ before invoking llama.cpp.
- C++ engine resources are RAII-managed and never exposed as raw pointers to JS.
- The Rust/C++ callback is synchronous and only borrows the Rust sink.
- Token bytes are UTF-8 validated on the Rust side.
- Token queues are bounded to apply backpressure.
- Model paths are accepted as local paths; higher layers should restrict them to OpenMindAI's configured model root before exposing model selection to untrusted clients.
- Do not expose the WebSocket example directly to the public internet without authentication, request limits, per-user generation concurrency, origin checks, and maximum prompt-size enforcement.

## Next integration step for the existing desktop app

After native-core is validated on Windows CPU + NVIDIA CUDA and the current desktop CI remains green, add an optional `native-llama` feature to `src-tauri/Cargo.toml` and a thin Tauri adapter that forwards channel tokens through `AppHandle::emit`. Keep the existing llama-server path as fallback until native GPU/driver compatibility has been exercised across supported hardware.
