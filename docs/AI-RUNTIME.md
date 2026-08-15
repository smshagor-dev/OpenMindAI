# AI Runtime

The runtime target is llama.cpp with GGUF models located beneath the project root `models` directory.

The current foundation includes:

- local model discovery for `.gguf`
- SQLite-backed model registry
- `LlamaRuntimeManager` process boundary
- localhost-only runtime design notes

The runtime manager must bind local services only to `127.0.0.1`, sanitize process arguments, kill child processes on shutdown, and avoid exposing llama.cpp details to the UI.

## Portable Runtime Layout

llama.cpp runtime payloads are stored under:

```text
runtimes/llama/
manifests/
cpu/<version>/
cuda12/<version>/
cuda13/<version>/
vulkan/<version>/
sycl/<version>/
hip/<version>/
active/
```

Runtime manifests use paths relative to OpenMindAI Root so drive letters can change.

## Milestone 2.1 Validation

The development machine staged the official llama.cpp Vulkan Windows x64 release:

```text
G:\portable ai\runtimes\llama\vulkan\b10414
```

Validated binaries:

- `llama-server.exe`
- `llama-cli.exe`
- `llama-bench.exe`

Probe output:

```text
version: 0.1.0-dev (build 10414, commit 4a84b0ad1)
built with Clang 20.1.8 for Windows x86_64
Available devices:
  Vulkan0: AMD Radeon RX 580 2048SP (8192 MiB, 7402 MiB free)
```

`llama-server.exe` was launched with `--host 127.0.0.1` and a dynamic/manual local port. `/health` returned `200` without loading a model. No large GGUF model was downloaded.

## Phase 2 Milestone 2 Validation

Validated model:

- Repository: `Qwen/Qwen3-4B-GGUF`
- Official file: `Qwen3-4B-Q4_K_M.gguf`
- Quantization: `Q4_K_M`
- Size: `2,497,280,256` bytes
- SHA256: `7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5`
- Local path: `models/llm/qwen/qwen3-4b/Qwen3-4B-Q4_K_M.gguf`
- Verification: GGUF header valid, metadata readable, SHA256 verified

Validated runtime:

- llama.cpp: build `10427`, commit `650913862`
- Backend: Vulkan
- Device: `Vulkan0: AMD Radeon RX 580 2048SP`
- Server bind: `127.0.0.1`
- Context tested: `8192`
- GPU layers requested: `999`
- GPU layers actually offloaded: `37/37`
- Flash Attention: disabled by the app launch plan; llama.cpp probe logs may still report auto/fused support when launched manually

Observed llama.cpp evidence:

```text
llama_prepare_model_devices: using device Vulkan0 (AMD Radeon RX 580 2048SP)
load_tensors: offloaded 37/37 layers to GPU
load_tensors: Vulkan0 model buffer size = 2375.91 MiB
llama_kv_cache: Vulkan0 KV buffer size = 1152.00 MiB
llama_context: n_ctx = 8192
llama_server: listening on http://127.0.0.1
```

Prompt validation logs under `logs/` show completed English, Bangla, coding, reasoning, cancellation, and post-cancel READY requests against the loaded model. Direct UI restart persistence was not fully re-exercised in this pass, so the milestone remains partial until a complete Tauri UI close/reopen workflow is captured.
