# Native llama.cpp inference bridge

OpenMindAI has an **optional**, feature-gated native llama.cpp inference core. The existing production chat/model router remains the default until routing, streaming, packaging, and runtime benchmarks are completed.

## Architecture boundary

`cxx` owns the Rust↔C++ boundary. Rust owns streaming callbacks and the C++ engine is held through `cxx::UniquePtr`, so model/context destruction follows RAII without exposing raw C pointers to application code.

The native core now provides:

- GGUF model loading with GPU layer selection at **engine load**
- model-native chat-template formatting through llama.cpp
- user and system prompts
- temperature/top-p/max-token configuration
- dynamic context, batch, and thread sizing
- reusable context allocation with KV reset between requests
- synchronous token streaming with callback cancellation
- serialized access to the mutable llama context
- a reusable Rust `InferenceBackend` abstraction

The feature is still opt-in:

```text
native-cxx-llama
```

## Current layout

```text
OpenMindAI/
├─ src-tauri/
│  ├─ native/
│  │  ├─ inference.h
│  │  └─ inference.cpp
│  └─ src/
│     ├─ inference.rs            # existing production HTTP/llama-server pipeline
│     ├─ native_inference.rs     # reusable native backend abstraction
│     ├─ native_bridge.rs
│     └─ ...existing Tauri/Rust core
├─ docs/
│  └─ NATIVE_INFERENCE_BRIDGE.md
└─ ...existing React/Tauri UI
```

The next application phase should route both the existing backend and this native backend behind one model router. Do not duplicate model ownership in React, Tauri commands, and C++.

## Build prerequisites

Normal builds do not require llama.cpp:

```bash
cargo build --manifest-path src-tauri/Cargo.toml
```

To enable native inference, build the pinned/compatible llama.cpp library and provide its include/source root and library directory:

```bash
export LLAMA_CPP_DIR=/absolute/path/to/llama.cpp
export LLAMA_CPP_LIB_DIR=/absolute/path/to/llama.cpp/build/bin
cargo build \
  --manifest-path src-tauri/Cargo.toml \
  --features native-cxx-llama
```

Windows PowerShell:

```powershell
$env:LLAMA_CPP_DIR = "C:\src\llama.cpp"
$env:LLAMA_CPP_LIB_DIR = "C:\src\llama.cpp\build\src\Release"
$env:PATH = "C:\src\llama.cpp\build\bin\Release;$env:PATH"
cargo build --manifest-path src-tauri\Cargo.toml --features native-cxx-llama
```

On Windows, `LLAMA_CPP_LIB_DIR` must contain the `llama.lib` import library.
The DLL directory belongs on `PATH` for execution and is staged into the
installer separately. The pinned Visual Studio build places these in
`build/src/Release` and `build/bin/Release`, respectively. Native builds validate
the import library before compiling the wrapper to avoid a late `LNK1181` failure.

The default linker mode is dynamic. Static builds may require explicit ggml libraries through:

```text
OPENMINDAI_LLAMA_LINK_KIND=static
OPENMINDAI_LLAMA_EXTRA_LIBS=ggml;ggml-base;ggml-cpu
```

## CUDA and CPU SIMD

The wrapper compiler flags do not enable llama.cpp kernels. CUDA, Vulkan/other ggml backends, AVX2/AVX512, and other optimized kernels must be enabled when llama.cpp itself is built.

Example CUDA build:

```bash
cmake -S "$LLAMA_CPP_DIR" -B "$LLAMA_CPP_DIR/build" \
  -DGGML_NATIVE=ON \
  -DGGML_CUDA=ON \
  -DBUILD_SHARED_LIBS=ON \
  -DLLAMA_BUILD_TESTS=OFF \
  -DLLAMA_BUILD_EXAMPLES=OFF
cmake --build "$LLAMA_CPP_DIR/build" --config Release -j
```

`OPENMINDAI_LLAMA_CUDA=1` tells the Rust build that the linked llama backend is CUDA-enabled and allows CUDA SDK library search paths to be added when available.

For portable release builds use:

```text
OPENMINDAI_PORTABLE_BUILD=1
```

and choose the CPU ISA baseline in the llama.cpp build.

## Model-native chat templates

The native engine reads the GGUF model's default chat template with `llama_model_chat_template` and formats system/user messages using `llama_chat_apply_template`.

This is intentionally different from hard-coding one global `System/User/Assistant` or ChatML layout. Qwen, Llama, Gemma, Mistral, and other models must receive the control tokens expected by their own template.

If a GGUF model has no chat template:

- a raw user prompt can still be used;
- a system prompt is rejected instead of being silently injected using an invented generic format.

If the template exists but is unsupported by the pinned llama.cpp build, generation returns an explicit error.

## Dynamic context policy

The target context is based on:

```text
max(configured_n_ctx, prompt_tokens + max_tokens + safety_margin)
```

and rounded to a small allocation block. Context allocation is reused while capacity and execution parameters remain suitable. The KV memory is cleared before each new request.

A mutex serializes generation and KV-clear operations on one engine because one mutable llama context must never be used concurrently.

Large project files should still use retrieval/chunking and bounded token budgets; dynamic KV sizing is not a substitute for context construction.

## Streaming and cancellation guarantees

llama.cpp token pieces cross CXX as byte slices. Rust validates UTF-8 and keeps an
incomplete code point until the next piece arrives, so Bengali, emoji, and other
multi-byte text are not damaged at token boundaries. Invalid byte sequences are
replaced with U+FFFD. An incomplete final character is replaced only on normal
completion, never after the consumer stops or generation fails.

The low-level callback can receive an empty string as a cancellation poll before
prompt batches and between generated tokens. Return `false` to stop. The desktop
adapter filters empty polls before its bounded token channel, UI, and database.
Stop wakes the receiver even before the first token and closes the channel before
waiting for the worker, releasing a producer blocked by backpressure. A request
guard cancels the worker and clears active conversation state on every exit,
including a dropped async request. Queued cancelled requests skip model loading.

Cancellation is cooperative between native calls; it cannot interrupt a model
load or a single running `llama_decode` call. No real-GPU inference or hard timeout
guarantee follows from compile/link and synthetic streaming tests.

## Rust API

The low-level bridge remains available:

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
};

engine.generate(
    "Explain this project",
    "You are a concise coding assistant.",
    config,
    |token| {
        print!("{token}");
        true
    },
)?;
```

GPU offload is intentionally configured only when the engine is loaded. It is not duplicated inside `GenerationConfig`.

Application code should prefer the reusable native backend boundary:

```rust
use open_mind_ai_lib::native_inference::{
    InferenceBackend, InferenceRequest, NativeBackend,
};

let mut backend = NativeBackend::load(std::path::Path::new("models/qwen.gguf"), -1)?;
let request = InferenceRequest::new(
    "Explain this project",
    "You are a concise coding assistant.",
);

backend.generate(request, Box::new(|token| {
    print!("{token}");
    true
}))?;
```

Generation is blocking. Async Tauri/Node orchestration must run it on a dedicated worker or blocking pool rather than blocking an async executor.

## Node.js / TypeScript integration

The recommended future boundary remains napi-rs:

```text
Next.js client
    │ WebSocket / SSE
    ▼
Node.js TypeScript server
    │ napi-rs ThreadsafeFunction / AsyncTask
    ▼
openmind-node
    │ Rust backend API
    ▼
openmind-core / InferenceBackend
    │ cxx
    ▼
inference.cpp -> llama.cpp -> hardware backend
```

Do not invoke V8/Node APIs directly from the inference thread. Use a threadsafe N-API callback and keep socket/session state in TypeScript.

## Validation

The dedicated native workflow pins llama.cpp commit:

```text
7798007a29a90e3053e799394da48cf53a2f8e0f
```

It builds a shared CPU llama library and then compiles, links, tests, and runs Clippy against an isolated smoke crate containing both the bridge and reusable native Rust inference module.

### Windows Vulkan package isolation

`Native Vulkan runtime` also downloads the staged DLL artifact on a fresh Windows
runner. `scripts/test-native-runtime.ps1` verifies the manifest commit, ABI tag,
file set, sizes and SHA256 hashes before running each loader scenario in a separate
PowerShell process with a 30-second timeout. No Vulkan SDK is installed by this job.
The probe removes SDK/build paths and Vulkan environment overrides. Its Windows
DLL search is restricted to the bundle directory and System32 using
[`LoadLibraryExW`](https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-loadlibraryexw).

The complete bundle must load, initialize llama.cpp, and allocate/free a 4 KiB CPU
backend buffer. Copies with required core DLLs removed must fail with Win32 error
126. For a dynamic Vulkan bundle, removing `ggml-vulkan.dll` must preserve CPU
initialization and allocation. A separate fault-injection copy changes the Vulkan
loader import name to a nonexistent DLL of the same byte length; CPU must still
work when that transitive dependency cannot load. CPU-only initialization must not
load Vulkan at all. Fresh child processes prevent an already-loaded DLL from
hiding a missing dependency. Loaded module paths are printed in CI logs.

The CI artifact also contains `native-backend-probe.exe`, built from the actual
Rust bridge and C++ wrapper. On the complete bundle and both damaged Vulkan copies,
CPU initialization must reach the expected nonexistent-GGUF error. GPU requests on
the damaged copies must return the explicit Vulkan-unavailable error before model
loading. This diagnostic executable is only added to the CI artifact, not staged
by the production packaging script. It tests backend startup without downloading a model.

Run the same check on a Windows x64 machine with PowerShell 7:

```powershell
./scripts/test-native-runtime.ps1 -RuntimeDir ./native-vulkan-artifact `
  -ExpectedCommit 7798007a29a90e3053e799394da48cf53a2f8e0f
```

This checks SDK-independent package loading with the host's system runtime/driver
libraries available. It does not prove that Vulkan runtime prerequisites are absent,
run a GGUF model, exercise GPU inference, or test the application's CPU retry.
The CPU buffer check only verifies that the CPU backend in the Vulkan bundle is usable.

### Optional Vulkan loading on Windows

The Vulkan validation build uses `-DGGML_BACKEND_DL=ON` so `llama.dll` and `ggml.dll`
do not require the Vulkan plugin at process startup. Build the OpenMindAI wrapper
with `OPENMINDAI_NATIVE_DYNAMIC_BACKENDS=1` and set `LLAMA_CPP_BACKEND_LIB_DIR` to
the directory containing the matching `ggml.lib` import library, in addition to
the existing `LLAMA_CPP_LIB_DIR` containing `llama.lib`. This mode currently supports
Windows MSVC. Stage that build with `prepare-native-runtime.ps1 -DynamicBackends`.
The manifest records `backendLoading: dynamic`; older manifests default to `linked`.
The application rejects a manifest whose loading mode does not match its build.

The native wrapper explicitly registers `ggml-cpu.dll` from the executable directory
before initializing llama.cpp. A development build can select one absolute directory
with `OPENMINDAI_NATIVE_BACKEND_DIR`. It never scans the current working directory,
PATH, or `GGML_BACKEND_PATH`. Plugin dependencies are preloaded using the plugin
directory and System32. Vulkan is attempted only when GPU layers are requested.
If the plugin, its loader, or a usable Vulkan device is unavailable, the wrapper
returns an error before loading the model or emitting tokens. The existing Rust
router then retries with zero GPU layers. Missing core/CPU libraries remain errors.

The normal release workflow still produces the linked CPU baseline. Dynamic Vulkan
packaging is validated separately before release enablement. This change handles
recoverable loading/availability failures, not native driver crashes or access
violations. Real-device GGUF generation, cancellation, and GPU-to-CPU recovery still
need validation before making Vulkan the default.

## Remaining production work

Before native inference becomes OpenMindAI's default chat path:

- wire `InferenceBackend` into the real model router;
- stream native tokens through the existing Tauri/React chat protocol;
- add request-scoped cancellation and lifecycle state;
- add invalid-model, OOM, long-context, multilingual, and repeated-generation runtime tests;
- add Windows/macOS native compile and packaging checks;
- validate CUDA and other supported GPU backends plus CPU fallback;
- solve DLL/rpath/dylib release packaging;
- benchmark model load, first-token latency, tokens/sec, RAM, VRAM, and cancellation latency;
- only then consider enabling native inference by default.
