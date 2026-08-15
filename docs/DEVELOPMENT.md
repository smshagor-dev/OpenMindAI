# Development

Install prerequisites:

- Node.js
- Rust and Cargo
- Tauri system dependencies for the target platform

Start the frontend:

```powershell
npm install
npm run dev
```

Start the desktop app:

```powershell
$env:OPENMINDAI_ROOT = "G:\portable ai"
npm run tauri dev
```

Or use the checked-in helper, which defaults `OPENMINDAI_ROOT` to a sibling
folder (`..\OpenMindAI-data`, never inside the repo itself) unless you pass
`-PortableRoot`:

```powershell
.\scripts\run-tauri-dev.ps1
```

Install an official llama.cpp runtime package into the portable root:

```powershell
.\scripts\install-llama-runtime.ps1 -Backend vulkan
```

Supported backend package folders are `cpu`, `vulkan`, `cuda12`, `cuda13`, `sycl`, and `hip`. The script downloads from official `ggml-org/llama.cpp` GitHub release artifacts and writes manifests under `runtimes\llama\manifests`.

Validation:

```powershell
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

Rust/Cargo must be installed before backend compilation and tests can run.

## Phase 2 Milestone 2 Validation Notes

Use `OPENMINDAI_ROOT=G:\portable ai` for local validation. The app should discover the installed Qwen model, register it with a relative path, plan Vulkan launch on the RX 580, and bind llama.cpp to `127.0.0.1`.

Current validated commands:

```powershell
cargo fmt --manifest-path src-tauri\Cargo.toml -- --check
cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri\Cargo.toml
npm run lint
npm run build
```

Observed model performance from manual llama.cpp server validation:

- English prompt: 22 prompt tokens at 87.46 tok/s, 192 generated tokens at 23.14 tok/s
- Bangla prompt: 84 prompt tokens at 104.47 tok/s, 192 generated tokens at 23.06 tok/s
- Coding prompt: 28 prompt tokens at 144.29 tok/s, 256 generated tokens at 22.91 tok/s
- Reasoning prompt: 34 prompt tokens at 66.38 tok/s, 102 generated tokens at 23.12 tok/s

Known validation gap: complete Tauri UI close/reopen persistence should be captured before marking Phase 2 Milestone 2 as PASS.
