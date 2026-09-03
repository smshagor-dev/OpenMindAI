# OpenMindAI

<p align="center">
  <strong>A local-first AI desktop workstation that runs useful AI on your own computer.</strong>
</p>

<p align="center">
  OpenMindAI combines local AI chat, model and runtime management, active project workspaces, local agents, connected apps, media tools, portable storage, and maintenance workflows in one desktop application. Core AI inference and local data stay under your control instead of depending on a recurring cloud AI subscription.
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

## OpenMindAI v3.0.0

**Current source version:** `v3.0.0`  
**Latest public release:** [v3.0.0](https://github.com/smshagor-dev/OpenMindAI/releases/tag/v3.0.0)

v3.0.0 moves OpenMindAI beyond local chat and model management into an active local AI workstation. Projects can now work with real folders, a local Project Agent can inspect and modify code, terminal-assisted workflows can detect failures and retry repairs, and connected services can be used from normal conversations without turning the application into a collection of raw API consoles.

### Download

| Platform | Installation path | Status |
| --- | --- | --- |
| Windows x64 | [OpenMindAI v3.0.0 installer](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/OpenMindAI_3.0.0_x64-setup.exe) | Primary packaged release |
| Windows | [Bootstrap setup](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/OpenMindAI-Setup.bat) | Available |
| Linux x64 | [Shell bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/openmindai-setup.sh) | Available; hardware coverage continues to expand |
| macOS | [Command bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/OpenMindAI-Setup.command) | Available; hardware coverage continues to expand |
| Linux ARM64 | — | Not currently supported |

Use the official [Releases](https://github.com/smshagor-dev/OpenMindAI/releases) page for release notes and downloadable assets.

### What changed in v3.0.0

- Projects now include active local workspaces instead of acting only as passive context containers.
- Existing folders can be opened directly as OpenMindAI Projects.
- Project Agent can inspect, create, edit, rename, and delete scoped project files.
- Project Agent can use explicitly permitted terminal commands, observe failures, repair changes, and validate the result.
- Git-aware status and diff inspection are available behind the Full PC + Terminal permission boundary.
- Local AI requests can use chat, thinking, vision, web search, and research-oriented workflows.
- Connected Apps integrate Google Workspace, GitHub, Microsoft 365, Slack, Notion, Dropbox, and MCP servers.
- Local document, image, video, voice, and sound-generation entry points are integrated with the artifact library.
- Model downloads, runtime installation, hardware-aware launch planning, diagnostics, backup, repair, and updater workflows remain part of the same desktop application.
- CI, security checks, release validation, installer workflows, and version-consistency gates were expanded for the v3 release line.

For the focused release summary, see [`.github/releases/v3.0.0.txt`](.github/releases/v3.0.0.txt).

## Why OpenMindAI

Many desktop AI tools still depend on a remote service for the part that matters most: inference. OpenMindAI takes a different approach. The application, conversations, models, runtime, projects, and generated working data can live on storage you control while the primary AI runtime executes locally on your machine.

The project is built around five practical goals:

- **Local-first operation** — core AI inference and chat history stay on the machine after setup.
- **Portable storage** — models and application data can live on a secondary drive, external SSD, or portable installation root.
- **Hardware-aware execution** — the application detects the host system and prepares a suitable local inference path instead of assuming identical hardware everywhere.
- **Active project work** — AI can work with real local projects and files while keeping explicit filesystem and terminal permission boundaries.
- **One desktop workflow** — chat, projects, models, connected apps, artifacts, diagnostics, updates, maintenance, and storage management live in one product.

## Core Capabilities

### Local AI chat

OpenMindAI runs supported open-weight language models locally and stores conversation history in a local SQLite database. Once the required model and runtime are installed, normal local chat does not require an internet connection.

The chat system supports streamed responses, cancellation, model selection, conversation history, message regeneration, editing, attachments, and local context persistence.

### Thinking, vision, search, and research modes

OpenMindAI can route different requests through specialized local flows. Image attachments can be passed to compatible local vision models, while Web Search and Deep Research modes can retrieve current public web evidence and provide it to the local model as external context.

Web retrieval is treated as untrusted data rather than executable instruction, and current-source claims should remain grounded in retrieved evidence.

### Model and runtime management

The application includes a local model catalog and lifecycle management for:

- model discovery;
- download and progress tracking;
- pause and cancellation controls;
- verification and validation;
- deletion;
- model activation per conversation;
- hardware-aware launch planning;
- local runtime installation and inventory;
- runtime start, stop, and health state;
- model update checks.

Large model downloads remain explicit user actions.

### Hardware-aware execution

OpenMindAI detects CPU, memory, and available graphics hardware and uses that profile when choosing runtime and model launch settings. The goal is graceful compatibility across different machines instead of assuming a single GPU vendor or fixed workstation specification.

When acceleration is unavailable, compatible CPU execution remains the fallback path.

## Projects and Project Agent

v3.0.0 makes Projects a first-class local development workspace.

A Project can contain:

- project instructions;
- linked conversations;
- reference/context files;
- an attached local folder;
- a local file browser and text editor;
- a Work surface for active project tasks;
- a linked Project Agent conversation.

### Open Folder as Project

An existing local folder can be opened and attached to a Project. OpenMindAI creates the durable project/chat relationship and keeps workspace access scoped to the attached folder by default.

This is intended for real applications, repositories, research code, websites, scripts, documents, and other local work rather than isolated demo folders.

### Local Project Agent

When a conversation belongs to a Project with an attached workspace, compatible chat and thinking requests can route to the Project Agent.

The agent can:

- inspect directories and files;
- read source code and text files;
- create and modify files;
- rename or delete scoped files when needed;
- use recent project conversation context;
- refresh workspace context after mutations;
- detect failed commands and timeouts;
- reason over failures and attempt repairs;
- run applicable validation before reporting completion;
- inspect hardened Git status and diff information when terminal access is enabled.

The agent uses the configured local OpenMindAI model/runtime rather than requiring a paid cloud coding model.

### Full PC + Terminal permission boundary

Filesystem and terminal access are deliberately separated from ordinary chat capability.

Attached project folders can be used as scoped workspaces. Broader **Full PC + Terminal** access must be explicitly approved before the project can execute terminal commands through that permission path. Direct local file mutations also use approval checks in the desktop backend.

The agent avoids interactive command flows and uses bounded execution, command-length limits, output limits, timeouts, duplicate-action protection, failure budgets, and validation requirements to reduce runaway behavior.

## Connected Apps

Connected services are optional. They extend OpenMindAI beyond the local machine but do not replace the local model/runtime architecture.

Supported app families include:

- **Google Workspace** — Gmail, Drive, Calendar, Contacts
- **GitHub** — repositories, files, issues, pull requests, Actions, releases
- **Microsoft 365** — Outlook, OneDrive, Calendar, Contacts
- **Slack**
- **Notion**
- **Dropbox**
- **MCP servers**

Connections are managed under **Settings → Apps**. Users interact through normal Chat or Project Work instead of selecting raw provider actions manually.

Connection secrets and OAuth tokens are kept out of chat history and stored through the operating-system credential path used by the application. Remote mutating actions remain subject to backend approval and provider permission checks.

See [`docs/CONNECTED_APPS.md`](docs/CONNECTED_APPS.md) for the connected-app UX and security contract.

## Artifacts and Media

OpenMindAI can create and track local artifacts associated with conversations.

Supported artifact paths include:

- plain text;
- Markdown;
- code files;
- PDF documents;
- DOCX documents;
- images;
- video;
- voice/audio;
- generated soundscape content.

Media capability depends on the required local model/runtime being installed and compatible with the current hardware. Missing dependencies are surfaced instead of silently switching to a paid cloud generation API.

Generated artifacts are tracked in the local library and can be opened or revealed from the desktop application.

## Architecture

OpenMindAI uses a web-based desktop interface with a native Rust backend.

```text
┌─────────────────────────────────────────────────────────────────┐
│                         OpenMindAI Desktop                      │
├─────────────────────────────────────────────────────────────────┤
│ React + TypeScript + Vite                                      │
│ Chat · Projects · Work · Models · Tools · Settings · Library   │
├─────────────────────────────────────────────────────────────────┤
│                           Tauri 2 IPC                           │
├─────────────────────────────────────────────────────────────────┤
│ Rust application core                                          │
│ Chat · Agents · Runtime · Storage · Files · Apps · Maintenance │
├───────────────────────────────┬─────────────────────────────────┤
│ SQLite / local app data       │ Local AI runtimes + model files │
└───────────────────────────────┴─────────────────────────────────┘
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
| Local LLM interface | llama-compatible OpenAI-style endpoint |
| Markdown/code rendering | Marked + Highlight.js |
| PDF handling | PDF.js + Rust document tooling |
| Speech/audio | Whisper + local TTS/audio tooling |
| Updates | Tauri updater |
| CI | GitHub Actions |
| Security scanning | CodeQL, `npm audit`, `cargo audit` |

## First Run

The first-run flow prepares a local AI environment rather than asking for a cloud AI account:

1. Install or launch OpenMindAI.
2. Choose where OpenMindAI AI data should live.
3. Create the local profile used by the application.
4. OpenMindAI detects available hardware.
5. The required local runtime is prepared.
6. Select or download a supported model.
7. The model is verified before it is treated as ready.
8. OpenMindAI opens into the desktop workspace.

Internet access is needed for initial source/runtime/model downloads and for features that explicitly use online services. Once the local components are present, local chat, history, settings, project work, diagnostics, and inference can continue without a cloud AI service.

## Storage and Portability

OpenMindAI separates the desktop application from the user's AI data. The selected storage root, referred to internally as `OPENMINDAI_ROOT`, can contain:

- model files;
- AI runtimes;
- the SQLite database;
- cache;
- logs;
- generated artifacts;
- workspaces;
- knowledge data;
- backups.

This makes it possible to keep a large AI environment on a secondary drive or external SSD and reduces the risk of losing models or history when the desktop application is updated or reinstalled.

### Portable mode

When an `openmindai.marker` file is present at the portable root, OpenMindAI can resolve its data location relative to that root. This avoids coupling a portable installation to one fixed Windows drive letter and makes external-drive workflows more reliable.

A configured storage location that is temporarily unavailable should be reported instead of silently replaced by a new empty data directory. An unplugged external drive must not look like lost history or a fresh installation.

## Default Local Model

The v3.0.0 baseline setup targets **Qwen3 4B (`Q4_K_M`)** as the default local language model.

Model weights are deliberately excluded from this repository. They are downloaded separately and stored under the selected AI data root, keeping multi-gigabyte model binaries out of normal Git history.

Actual inference performance depends on CPU, GPU, available memory, model size, quantization, context length, and backend support.

## Offline and Online Behavior

OpenMindAI is local-first, not network-blind. Features that genuinely require external data still use a network connection.

### Designed to work offline after local setup

- local AI inference;
- normal chat and conversation history;
- local settings and profile data;
- Projects and attached local workspaces;
- Project Agent file operations;
- installed model/runtime management;
- local document and supported media generation;
- diagnostics, maintenance, and local backups.

### Requires network access when used

- initial bootstrap/source downloads;
- runtime downloads;
- model downloads;
- application and model update checks;
- Web Search and Deep Research retrieval;
- Google Workspace, GitHub, Microsoft 365, Slack, Notion, Dropbox, and MCP connections where applicable.

A temporary inability to reach GitHub or another external provider should not make an already-prepared local AI workstation unusable for its offline capabilities.

## Development

### Requirements

- **Node.js 24** (the version used by CI)
- npm
- a current stable Rust toolchain
- platform-specific Tauri build dependencies

Windows development requires the Microsoft C++/MSVC toolchain used by Tauri. Linux requires the WebKitGTK/AppIndicator development packages expected by Tauri 2.

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

The application version is synchronized across `package.json`, the Rust crate, and the Tauri configuration. `npm run check:version` is part of the validation path so version drift is caught before release work.

## Continuous Integration

The main CI workflow validates both the web frontend and native Rust application.

### Frontend

- clean dependency installation with `npm ci`;
- release-version consistency check;
- ESLint;
- TypeScript compilation;
- Vite production build.

### Rust

- Linux, Windows, and macOS runners;
- `rustfmt` check;
- Clippy with warnings treated as errors;
- Rust test suite with all features enabled.

See [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

Additional workflows cover project-workspace preflight, local-agent checks, release readiness, security, and release packaging.

## Security

OpenMindAI gives a local AI system meaningful access to files, optional terminal commands, and connected services, so permission boundaries are part of the product rather than documentation-only guidance.

Key controls include:

- explicit Tauri capability configuration;
- a restrictive desktop Content Security Policy;
- scoped project folder access by default;
- explicit approval for Full PC + Terminal access;
- approval guards for direct workspace mutations and remote mutating actions;
- bounded agent steps, failures, repeated actions, command length, output, and timeouts;
- validation requirements after workspace changes;
- secret/token storage outside conversation history;
- external provider data treated as untrusted input;
- local model/runtime binaries excluded from source control;
- `npm audit`, `cargo audit`, and CodeQL security checks.

See [`SECURITY.md`](SECURITY.md) and [`.github/workflows/security.yml`](.github/workflows/security.yml).

## Release Pipeline

Tagged releases use the dedicated release workflow in [`.github/workflows/release.yml`](.github/workflows/release.yml).

The release path checks version consistency, application builds, installer output, updater configuration, release metadata, and other release-readiness requirements. Signing and trust requirements are handled by the production release workflow rather than being embedded in source code or committed secrets.

For v3.0.0 release expectations and validation scope, see [`.github/releases/v3.0.0.txt`](.github/releases/v3.0.0.txt).

## Platform Notes

### Windows

Windows x64 is the primary packaged and tested platform for v3.0.0. The desktop release uses a Tauri NSIS installer. GPU/runtime behavior depends on installed hardware and backend support, while CPU execution remains the compatibility path when acceleration is unavailable.

### Linux

The Linux bootstrap supports the source/bootstrap installation path and the Linux dependency path is exercised by CI. Desktop environments and GPU stacks vary considerably, so hardware-specific validation continues to expand.

### macOS

The macOS bootstrap supports the source/bootstrap path and handles Intel/Apple Silicon environments. Distribution signing and notarization are separate concerns from source-level compatibility.

## Troubleshooting

### Setup cannot download a model

Check network access and available space in the selected AI storage root, then retry. Interrupted large downloads should not require deleting the entire installation.

### The configured storage drive is missing

Reconnect the drive or restore the configured location. OpenMindAI should not silently create a second unrelated data root because that can make existing history and models appear to have disappeared.

### GPU acceleration is unavailable

Use a compatible fallback backend where available. Local CPU inference is slower but remains the broad compatibility path for supported models.

### Project Agent cannot run terminal commands

Terminal execution is intentionally disabled until **Full PC + Terminal** access is explicitly enabled for the project. Normal scoped project file access does not automatically grant terminal permission.

### A connected app is unavailable

Open **Settings → Apps** and confirm that the provider is configured and connected. Some providers require OAuth/client configuration for this self-hosted desktop application.

### Windows shows a SmartScreen warning

Confirm that the installer came from the official OpenMindAI GitHub release. Release trust and signing state can vary between development and production artifacts, so do not use installers from third-party mirrors.

### A source build fails on a new machine

Confirm Node.js, Rust, and the platform-specific Tauri prerequisites first. Then run the frontend and Rust validation commands separately to determine whether the failure belongs to the web frontend, native toolchain, or operating-system dependency layer.

## Repository Policy

This repository contains source code, build configuration, bootstrap tooling, documentation, workflow configuration, and small project assets. Machine-local runtime state does not belong in Git.

Do not commit:

- local SQLite databases;
- downloaded model weights;
- AI runtimes;
- generated content;
- project workspaces;
- local caches and logs;
- backups;
- OAuth tokens or application secrets;
- signing keys;
- large model formats such as GGUF or SafeTensors.

## Contributing

OpenMindAI is under active development. Useful contributions include reproducible bug reports, platform validation, performance measurements, security improvements, documentation fixes, connector improvements, and focused code changes.

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

Please keep pull requests focused and do not commit local runtime data, model binaries, credentials, or user project data.

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for contribution guidance.

## License

OpenMindAI is released under the [Apache License 2.0](LICENSE).

Third-party components and attribution notices are documented in [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt).

## Maintainer

**Md Shahanur Islam Shagor**  
Full-Stack Web Developer & AI Engineer

- GitHub: [@smshagor-dev](https://github.com/smshagor-dev)
- Portfolio: [smshagor.com](https://smshagor.com)

---

If OpenMindAI is useful to you, consider starring the repository. It helps other developers find the project and supports continued work on practical, user-controlled local AI software.

### Native service and resource limits

The Go API can now launch the Rust/CXX worker directly with
`OPENMINDAI_API_BACKEND=native`. This optional local mode supports text chat and
SSE, keeps model paths in a local registry, and restarts the worker after a
cancelled or failed request. See [native service setup](services/native-worker/README.md)
for build commands, request fields, resource bounds and validation coverage.

Native inference also enforces context and KV admission limits and generation
deadlines. A stalled desktop native call quarantines the worker until restart.
Production Vulkan/default-native rollout still requires real device validation;
CPU recovery tests do not establish RX580 performance or reliability.

The optional [personalization CLI](experiments/python/README.md) trains local
LoRA adapters from approved corrections, evaluates held-out prompts, converts
to GGUF and checks native generation before explicit activation. Versioned
pointers support rollback in the Go worker and enabled native desktop chat.
Synthetic CPU integration covers this pipeline; it does not certify quality or
memory requirements for a user's Qwen3 model or add automatic background learning.
