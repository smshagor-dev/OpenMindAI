# OpenMindAI

<p align="center">
  <strong>A local-first AI desktop workstation built to run useful AI on your own computer.</strong>
</p>

<p align="center">
  OpenMindAI brings local chat, model management, projects, tools, hardware-aware inference, and portable storage into one desktop application. After the initial model/runtime setup, the core experience works without a cloud account, API key, or recurring AI subscription.
</p>

<p align="center">
  <a href="https://github.com/smshagor-dev/OpenMindAI/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/smshagor-dev/OpenMindAI/actions/workflows/ci.yml/badge.svg?branch=main"></a>
  <a href="https://github.com/smshagor-dev/OpenMindAI/actions/workflows/security.yml"><img alt="Security" src="https://github.com/smshagor-dev/OpenMindAI/actions/workflows/security.yml/badge.svg?branch=main"></a>
  <a href="https://github.com/smshagor-dev/OpenMindAI/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/smshagor-dev/OpenMindAI?display_name=tag&sort=semver"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/github/license/smshagor-dev/OpenMindAI"></a>
  <a href="https://github.com/smshagor-dev/OpenMindAI"><img alt="Platform" src="https://img.shields.io/badge/desktop-Windows%20%7C%20Linux%20%7C%20macOS-informational"></a>
</p>

<p align="center">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-2021-000000?logo=rust&logoColor=white">
  <img alt="TypeScript" src="https://img.shields.io/badge/TypeScript-5.x-3178C6?logo=typescript&logoColor=white">
  <img alt="React" src="https://img.shields.io/badge/React-18-61DAFB?logo=react&logoColor=111111">
  <img alt="Tauri" src="https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white">
  <img alt="Vite" src="https://img.shields.io/badge/Vite-6-646CFF?logo=vite&logoColor=white">
  <img alt="SQLite" src="https://img.shields.io/badge/SQLite-local-003B57?logo=sqlite&logoColor=white">
</p>

---

## Why OpenMindAI

Most desktop AI tools still depend on a remote service for the part that matters most: inference. OpenMindAI takes a different approach. The application, conversation history, model files, runtime, projects, and generated working data live on storage you control, while the AI runtime executes locally on your machine.

The project is designed around four practical goals:

- **Local-first operation** — core AI inference and chat history stay on the machine after setup.
- **Portable storage** — models and application data can live on a secondary drive, external SSD, or portable installation root.
- **Hardware-aware execution** — OpenMindAI detects the host system and selects an appropriate local inference path instead of assuming every computer has the same GPU.
- **A complete desktop workflow** — chat is only one part of the application; models, projects, files, tools, diagnostics, updates, and maintenance belong in the same product.

## Current Release

**Latest public release: [v3.0.0](https://github.com/smshagor-dev/OpenMindAI/releases/tag/v3.0.0)**

The v3.0.0 release includes the Windows installer together with bootstrap installers for Windows, Linux, and macOS.

Version 3 turns Projects into an active local development workspace: a folder can be opened as a project, the local agent can inspect and edit files, run permitted terminal commands, observe failures, retry repairs, validate changes, and inspect Git state while preserving the explicit Full PC + Terminal permission boundary.

| Platform | Installation path | Current status |
| --- | --- | --- |
| Windows x64 | [OpenMindAI v3.0.0 installer](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/OpenMindAI_3.0.0_x64-setup.exe) | Primary tested platform |
| Windows | [Git bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/OpenMindAI-Setup.bat) | Available |
| Linux x64 | [Shell bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/openmindai-setup.sh) | Implemented; broader hardware validation in progress |
| macOS | [Command bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/OpenMindAI-Setup.command) | Implemented; broader hardware validation in progress |
| Linux ARM64 | — | Not currently supported |

For release notes and downloadable assets, use the [Releases](https://github.com/smshagor-dev/OpenMindAI/releases) page rather than third-party mirrors.

> The currently published Windows build can still trigger Windows SmartScreen on systems that do not recognize the publisher. Download releases only from this repository and verify published checksums when available.

## What It Can Do

### Local AI chat

OpenMindAI runs open-weight language models locally and keeps conversation history in a local SQLite database. Once the runtime and model are installed, normal chat does not require an internet connection.

### Model management

The application provides a model catalog and local model lifecycle controls, including download state, progress, verification, cancellation, deletion, and hardware-aware recommendations. Large model downloads remain explicit user actions.

### Automatic model routing

Requests can be routed internally to an appropriate local capability based on the task. The routing layer is intended to keep the interface simple while allowing different models or runtimes to handle chat, code, files, image, voice, and other specialized workloads.

### Projects and local context

Projects can group conversations, instructions, and local files. A local folder can also be opened directly as a project. Project Agent uses the attached workspace as live context and can read, create, edit, rename, or delete scoped files. When the user explicitly enables Full PC + Terminal access, the agent can run non-interactive shell commands and hardened Git status/diff inspection, recover from non-zero command failures, refresh its workspace snapshot after edits, and require appropriate validation before reporting a changed workspace as complete. Text and code files can become project context, while larger or binary files remain tracked as local project resources.

### Tools and maintenance

OpenMindAI includes workspace-level tools for models, files, diagnostics, local library workflows, maintenance, backup, and repair operations. These are kept inside the desktop application instead of being split across separate utilities.

### Media capability entry points

The UI includes local image, video, and voice generation entry points. Availability depends on whether the required model and runtime are installed on the machine. OpenMindAI reports missing local dependencies instead of silently falling back to a paid cloud API.

## How It Is Built

OpenMindAI uses a web-based desktop UI with a native Rust backend.

```text
┌──────────────────────────────────────────────────────────────┐
│                       OpenMindAI Desktop                     │
├──────────────────────────────────────────────────────────────┤
│ React + TypeScript + Vite                                   │
│ Chat · Models · Projects · Tools · Settings                 │
├──────────────────────────────────────────────────────────────┤
│                         Tauri 2 IPC                          │
├──────────────────────────────────────────────────────────────┤
│ Rust application core                                       │
│ Installation · Runtime · Storage · Updates · Files · System │
├─────────────────────────────┬────────────────────────────────┤
│ SQLite / local app data     │ Local AI runtime + model files │
└─────────────────────────────┴────────────────────────────────┘
```

### Main technologies

| Area | Technology |
| --- | --- |
| Desktop shell | Tauri 2 |
| Native backend | Rust 2021 |
| Frontend | React 18 + TypeScript |
| Build tooling | Vite 6 |
| Local database | SQLite via `rusqlite` |
| Async/runtime work | Tokio |
| Networking/downloads | Reqwest |
| Markdown/code rendering | Marked + Highlight.js |
| PDF handling | PDF.js / Rust document tooling |
| Updates | Tauri updater |
| CI | GitHub Actions |
| Security scanning | CodeQL, `npm audit`, `cargo audit` |

## First Run

The setup flow is intentionally different from a typical cloud AI application:

1. Install or launch OpenMindAI.
2. Choose where AI data should live.
3. Create the local profile used by the application.
4. OpenMindAI detects the available hardware.
5. The required local runtime is prepared.
6. Select or download a supported model.
7. The model is verified before it is treated as ready.
8. OpenMindAI opens into the local workspace.

Internet access is needed for the initial source/runtime/model downloads and for update checks. Once the required local components are present, chat, history, settings, local projects, diagnostics, and inference can continue offline.

## Storage and Portability

OpenMindAI separates the application from the user's AI data. The selected storage root, referred to internally as `OPENMINDAI_ROOT`, can contain model files, runtimes, the local database, cache, logs, generated files, workspaces, knowledge data, and backups.

This matters for two reasons:

- reinstalling or updating the application does not require treating model/data storage as disposable;
- a large AI library can be placed on a drive with enough capacity instead of consuming the system drive by default.

### Portable mode

When an `openmindai.marker` file is present at the portable root, OpenMindAI can resolve its data location relative to that root. This avoids coupling a portable installation to a fixed Windows drive letter and makes external-drive workflows more reliable.

A missing or unavailable configured storage location should be reported rather than silently replaced with a new empty data directory. That behavior is important because an unplugged external drive must not look like lost chat history or a fresh installation.

## Default Model

The default v3.0.0 setup targets **Qwen3 4B (`Q4_K_M`)** as the baseline local language model.

Model weights are deliberately excluded from the Git repository. They are downloaded separately and stored in the user's selected AI data root. This keeps the repository source-focused and prevents multi-gigabyte model binaries from entering normal Git history.

Actual inference speed depends on CPU, GPU, memory, model size, quantization, context length, and the selected backend.

## Development

### Requirements

- **Node.js 24** (the version used by CI)
- npm
- a current stable Rust toolchain
- platform-specific Tauri build dependencies

Windows development also requires the normal Microsoft C++/MSVC build tooling used by Tauri. Linux requires the WebKitGTK/AppIndicator development packages expected by Tauri 2.

### Run locally

```bash
git clone https://github.com/smshagor-dev/OpenMindAI.git
cd OpenMindAI
npm ci
npm run tauri dev
```

### Frontend validation

```bash
npm run check:version
npm run lint
npm run build
```

### Rust validation

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-features
```

### Production build

```bash
npm run build
npm run tauri -- build
```

The package version is intentionally synchronized across the JavaScript package, Rust crate, and Tauri configuration. `npm run check:version` is part of CI so version drift is caught before release work.

## Continuous Integration

The main CI workflow runs on pushes and pull requests and validates both halves of the application.

**Frontend validation**

- clean dependency install with `npm ci`
- release-version consistency check
- ESLint
- TypeScript compilation
- Vite production build

**Rust validation**

- Linux, Windows, and macOS runners
- `rustfmt` check
- Clippy with warnings treated as errors
- Rust test suite with all features enabled

The workflow lives at [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

## Security Checks

Security validation is handled separately from the normal build pipeline.

- `npm audit` blocks high-severity frontend dependency findings.
- `cargo audit` checks the Rust dependency lockfile.
- GitHub CodeQL analyzes JavaScript/TypeScript and Rust.
- The security workflow runs on pushes, pull requests, and a scheduled weekly scan.
- The Tauri application defines an explicit Content Security Policy rather than leaving the desktop WebView unrestricted.
- Model files and runtime payloads are kept outside normal source control.

See [`.github/workflows/security.yml`](.github/workflows/security.yml) for the exact checks.

## Release Pipeline

Tagged releases use a dedicated GitHub Actions workflow. The production Windows release path validates synchronized versions, requires updater-signing and Windows code-signing secrets, builds the Tauri release, prepares updater metadata, and generates SHA-256 checksums for release bundles.

That release path is intentionally strict: missing signing material causes the production release job to fail instead of producing an artifact that appears fully release-ready.

See [`.github/workflows/release.yml`](.github/workflows/release.yml).

## Offline Behavior

OpenMindAI distinguishes between operations that genuinely need a network and operations that should not.

**Requires network access**

- first-time source/bootstrap downloads
- runtime downloads
- model downloads
- release/update checks

**Designed to work offline after setup**

- local AI inference
- conversation history
- local settings
- project data already stored on the machine
- installed model/runtime management
- diagnostics, maintenance, and local backup workflows

A temporary inability to reach GitHub should not make an already-installed local AI workstation unusable.

## Platform Notes

### Windows

Windows x64 is the primary tested platform. The project uses an NSIS-based Tauri installer. GPU/runtime behavior depends on the available hardware and backend support; CPU operation remains the compatibility path when acceleration is unavailable.

### Linux

The Linux bootstrap handles common package-manager families and the Tauri/Linux dependency path is also exercised by CI. Real-world desktop environments and GPU stacks still vary significantly, so Linux should be treated as a platform under active validation rather than assumed identical to Windows.

### macOS

The bootstrap handles Intel and Apple Silicon detection. macOS builds require the normal Apple development/runtime prerequisites. Signing and notarization requirements apply to distributed macOS applications and should be evaluated separately from source-level compatibility.

## Troubleshooting

### Setup cannot download a model

Check network access and free space in the selected AI storage root, then retry. Interrupted large downloads should not require deleting the entire installation.

### The configured storage drive is missing

Reconnect the drive or restore the configured location. OpenMindAI should not silently create a second unrelated data root, because that can make existing history and models appear to have disappeared.

### GPU acceleration is unavailable

Local AI can still run through a compatible fallback backend, including CPU execution where supported. Performance will be lower than a suitable accelerated backend.

### Windows shows a SmartScreen warning

Make sure the file came from the official GitHub release. When a checksum is published for the asset, verify it before running the installer.

### A source build fails on a new machine

Confirm Node.js, Rust, and the platform-specific Tauri prerequisites are installed first. Then run the frontend and Rust validation commands above separately; this usually identifies whether the failure is in the web frontend, native toolchain, or operating-system dependencies.

## Repository Policy

This repository is for source code, build configuration, bootstrap tooling, and small project assets. Runtime state does not belong in Git.

In particular, local databases, downloaded models, AI runtimes, caches, logs, generated content, workspaces, backups, and other user data should remain outside version control. Large model formats such as GGUF and SafeTensors must not be committed to the repository.

## Contributing

OpenMindAI is under active development. Useful contributions include reproducible bug reports, platform validation, performance measurements, security improvements, documentation fixes, and focused code changes.

Before opening a pull request, run the same core checks used by CI:

```bash
npm ci
npm run check:version
npm run lint
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-features
```

Please keep pull requests focused and avoid committing local runtime data or model binaries.

## License

OpenMindAI is released under the [Apache License 2.0](LICENSE).

Third-party components and attribution notices are documented in [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt).

## Maintainer

**Md Shahanur Islam Shagor**  
Full-Stack Web Developer & AI Engineer

- GitHub: [@smshagor-dev](https://github.com/smshagor-dev)
- Portfolio: [smshagor.com](https://smshagor.com)

---

If OpenMindAI is useful to you, consider starring the repository. It helps make the project easier to discover and gives the work a clear signal that local, user-controlled AI software is worth continuing to build.
