# OpenMindAI

<p align="center">
  <strong>Local-first AI for desktop, now expanding to mobile.</strong>
</p>

<p align="center">
  OpenMindAI is a user-controlled AI workstation built around local inference, local data, portable model storage, active project work, connected apps, and offline-capable workflows. The stable desktop application remains the primary public release, while a separately maintained Flutter mobile application is now available as an unofficial development preview line.
</p>

<p align="center">
  <a href="https://github.com/smshagor-dev/OpenMindAI/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/smshagor-dev/OpenMindAI/actions/workflows/ci.yml/badge.svg?branch=main"></a>
  <a href="https://github.com/smshagor-dev/OpenMindAI/actions/workflows/security.yml"><img alt="Security" src="https://github.com/smshagor-dev/OpenMindAI/actions/workflows/security.yml/badge.svg?branch=main"></a>
  <a href="https://github.com/smshagor-dev/OpenMindAI/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/smshagor-dev/OpenMindAI?display_name=tag&sort=semver"></a>
  <img alt="Mobile preview" src="https://img.shields.io/badge/mobile-v3.1.0--unofficial.1-orange">
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/github/license/smshagor-dev/OpenMindAI"></a>
</p>

<p align="center">
  <img alt="Desktop" src="https://img.shields.io/badge/Desktop-Windows%20%7C%20Linux%20%7C%20macOS-informational">
  <img alt="Mobile" src="https://img.shields.io/badge/Mobile-Android%20%7C%20iOS-informational">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-2021-000000?logo=rust&logoColor=white">
  <img alt="TypeScript" src="https://img.shields.io/badge/TypeScript-5.x-3178C6?logo=typescript&logoColor=white">
  <img alt="Flutter" src="https://img.shields.io/badge/Flutter-Mobile-02569B?logo=flutter&logoColor=white">
  <img alt="Tauri" src="https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white">
  <img alt="SQLite" src="https://img.shields.io/badge/SQLite-local-003B57?logo=sqlite&logoColor=white">
</p>

---

## Release status

| Product line | Version | Status |
| --- | --- | --- |
| OpenMindAI Desktop | `v3.0.0` | Stable public release |
| OpenMindAI Mobile | `v3.1.0-unofficial.1` | Unofficial development preview / pre-release |

The mobile preview label intentionally does **not** replace the stable desktop `v3.0.0` application version. If a mobile binary is published under `v3.1.0-unofficial.1`, it should be treated as a GitHub **pre-release**, not as a production-ready mobile release.

For the mobile preview release summary, see [`.github/releases/v3.1.0-unofficial.1.txt`](.github/releases/v3.1.0-unofficial.1.txt).

---

## OpenMindAI Mobile — Unofficial Preview

**Preview label:** `v3.1.0-unofficial.1`  
**Platforms:** Android and iOS  
**Status:** Unofficial / pre-release / active development  
**Architecture:** Flutter + local llama.cpp runtime  
**Public source policy:** The mobile application is maintained separately and is not included in this public desktop source repository.

OpenMindAI Mobile brings the same local-first philosophy to phones and tablets: model files are downloaded to the device, inference runs locally through llama.cpp, and ordinary local chat does not require a paid model API.

### Current mobile capabilities

- Local chat with streamed responses.
- Chat and Thinking modes with model-aware prompting.
- Local model switching using canonical model IDs.
- Hardware-aware model recommendations based on RAM, storage, and CPU architecture.
- Model Manager grouping by provider.
- Compatibility states for **Compatible**, **High memory requirement**, **Not recommended**, **Insufficient storage**, and **Unsupported**.
- Resumable downloads using `.part` files.
- Real download percentage, transferred bytes, total size, and download speed.
- Retry and cancellation support for model downloads.
- Size and install validation before a model is considered ready.
- Direct-native llama.cpp inference with a local llama server fallback path.
- Automatic fallback to another compatible installed model when appropriate.
- Local vision model support through OpenMindAI Lens.
- Local attachment/document context workflows.
- No paid inference API requirement for supported local models.

### Mobile local model pack

The current mobile catalog includes the following families:

| Provider | Models |
| --- | --- |
| OpenMindAI / Alibaba Qwen | OpenMindAI Nano — Qwen3 0.6B, OpenMindAI Swift — Qwen3 1.7B, OpenMindAI Core — Qwen3 4B, OpenMindAI Titan — Qwen3 8B |
| OpenMindAI Vision | OpenMindAI Lens — Qwen2.5-VL 3B |
| DeepSeek | DeepSeek R1 Distill Qwen 1.5B, DeepSeek R1 Distill Qwen 7B |
| OpenAI | GPT OSS 20B, GPT OSS 120B |
| Google | Gemma 3 1B, Gemma 3 4B, Gemma 3 12B, Gemma 3 27B |
| Microsoft | Phi-4 Mini Instruct |
| Hugging Face | SmolLM3 3B |
| IBM | Granite 3.3 2B Instruct, Granite 3.3 8B Instruct |
| Meta | Llama 3.2 1B Instruct, Llama 3.2 3B Instruct |
| Mistral AI | Mistral 7B Instruct v0.3, Mistral Small 3.1 24B |
| AI2 | OLMo 2 7B Instruct |

Model Manager does not assume that every visible model is suitable for every phone. Large or license-sensitive models can remain visible for manual installation while being excluded from normal automatic recommendations.

Examples include GPT OSS 120B, Gemma 27B, Mistral Small 24B, and other memory-heavy models that are unrealistic for ordinary mobile devices.

### Mobile runtime and routing

The mobile application uses one canonical model architecture from install through inference:

```text
MobileModelCatalog
      ↓
Device compatibility
      ↓
Model Manager / Onboarding
      ↓
Hugging Face artifact resolution
      ↓
Resumable download + validation
      ↓
App-private model storage
      ↓
Model router
      ↓
lib_llama_cpp / llama.cpp
      ↓
Local streamed reply
```

The runtime includes model-specific handling where needed. Qwen3 supports `/think` and `/no_think`; SmolLM3 receives its reasoning-mode directive and recommended sampling behavior; reasoning-focused families receive appropriate generation budgets; and Mistral 7B v0.3 receives prompt normalization because its chat template does not natively accept the same system-role shape as every other model family.

### Android device coverage

The Android build packages the local llama runtime for these supported ABIs:

- `armeabi-v7a`
- `arm64-v8a`
- `x86_64`

This covers modern ARM phones/tablets, supported 32-bit ARM targets, and x86_64 Android emulator environments. The mobile CI checks that the packaged APK contains the llama native library for all three supported ABIs.

The application still performs device-level checks because ABI support alone does not mean a model will fit in memory or available storage.

### Mobile licensing note

The OpenMindAI source repository uses Apache-2.0, but downloaded model weights keep their own upstream licenses and usage terms.

Examples:

- GPT OSS — Apache-2.0 upstream model terms.
- Phi-4 Mini — MIT upstream model terms.
- Granite and many Mistral/SmolLM entries — Apache-2.0 upstream terms.
- Gemma — Google Gemma terms.
- Meta Llama — Llama Community License.
- OLMo and other models with additional upstream conditions remain subject to those terms.

OpenMindAI surfaces model license metadata rather than presenting all model weights as if they shared the desktop application's Apache-2.0 source license.

---

## OpenMindAI Desktop v3.0.0

**Current desktop source version:** `v3.0.0`  
**Latest stable public release:** [v3.0.0](https://github.com/smshagor-dev/OpenMindAI/releases/tag/v3.0.0)

v3.0.0 moves OpenMindAI beyond local chat and model management into an active local AI workstation. Projects can work with real folders, a local Project Agent can inspect and modify code, terminal-assisted workflows can detect failures and retry repairs, and connected services can be used from normal conversations without turning the application into a collection of raw API consoles.

### Desktop download

| Platform | Installation path | Status |
| --- | --- | --- |
| Windows x64 | [OpenMindAI v3.0.0 installer](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/OpenMindAI_3.0.0_x64-setup.exe) | Primary packaged release |
| Windows | [Bootstrap setup](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/OpenMindAI-Setup.bat) | Available |
| Linux x64 | [Shell bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/openmindai-setup.sh) | Available; hardware coverage continues to expand |
| macOS | [Command bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/v3.0.0/OpenMindAI-Setup.command) | Available; hardware coverage continues to expand |
| Linux ARM64 | — | Not currently supported |

Use the official [Releases](https://github.com/smshagor-dev/OpenMindAI/releases) page for stable desktop release notes and downloadable assets.

### What changed in desktop v3.0.0

- Projects now include active local workspaces instead of acting only as passive context containers.
- Existing folders can be opened directly as OpenMindAI Projects.
- Project Agent can inspect, create, edit, rename, and delete scoped project files.
- Project Agent can use explicitly permitted terminal commands, observe failures, repair changes, and validate the result.
- Git-aware status and diff inspection is available behind the Full PC + Terminal permission boundary.
- Local AI requests can use chat, thinking, vision, web search, and research-oriented workflows.
- Connected Apps integrate Google Workspace, GitHub, Microsoft 365, Slack, Notion, Dropbox, and MCP servers.
- Local document, image, video, voice, and sound-generation entry points are integrated with the artifact library.
- Model downloads, runtime installation, hardware-aware launch planning, diagnostics, backup, repair, and updater workflows remain part of the same desktop application.
- CI, security checks, release validation, installer workflows, and version-consistency gates were expanded for the v3 release line.

For the stable desktop release summary, see [`.github/releases/v3.0.0.txt`](.github/releases/v3.0.0.txt).

---

## Why OpenMindAI

Many AI applications still depend on a remote service for the part that matters most: inference. OpenMindAI takes a different approach. Conversations, models, runtimes, projects, and generated working data can live on storage you control while supported AI inference runs locally on your own hardware.

The project is built around these practical goals:

- **Local-first operation** — core AI inference and history can stay on the device after setup.
- **Portable storage** — desktop models and application data can live on a secondary drive or external SSD.
- **Hardware-aware execution** — model/runtime selection accounts for the current device instead of assuming identical hardware everywhere.
- **Active project work** — desktop AI can work with real local projects and files while keeping explicit filesystem and terminal permission boundaries.
- **Cross-device direction** — the desktop workstation and mobile companion follow the same privacy-first, local-model design philosophy.
- **No recurring inference subscription requirement** — supported local models run without a paid inference API.

---

## Desktop core capabilities

### Local AI chat

OpenMindAI Desktop runs supported open-weight language models locally and stores conversation history in a local SQLite database. Once the required model and runtime are installed, normal local chat does not require an internet connection.

The desktop chat system supports streamed responses, cancellation, model selection, conversation history, message regeneration, editing, attachments, and local context persistence.

### Thinking, vision, search, and research modes

OpenMindAI can route different requests through specialized flows. Image attachments can be passed to compatible local vision models, while Web Search and Deep Research modes can retrieve current public web evidence and provide it to the local model as external context.

Web retrieval is treated as untrusted data rather than executable instruction.

### Model and runtime management

The desktop application includes lifecycle management for:

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

OpenMindAI detects CPU, memory, and available graphics hardware and uses that profile when choosing runtime and model launch settings. When acceleration is unavailable, compatible CPU execution remains the broad fallback path.

---

## Projects and Project Agent

Desktop v3.0.0 makes Projects a first-class local development workspace.

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

Attached project folders can be used as scoped workspaces. Broader **Full PC + Terminal** access must be explicitly approved before the project can execute terminal commands through that permission path.

The agent uses bounded execution, command-length limits, output limits, timeouts, duplicate-action protection, failure budgets, and validation requirements to reduce runaway behavior.

---

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

Connections are managed under **Settings → Apps**. Connection secrets and OAuth tokens are kept out of chat history and stored through the operating-system credential path used by the application.

See [`docs/CONNECTED_APPS.md`](docs/CONNECTED_APPS.md) for the connected-app UX and security contract.

---

## Artifacts and Media

OpenMindAI Desktop can create and track local artifacts associated with conversations, including:

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

---

## Architecture

### Desktop

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

### Mobile preview

```text
┌──────────────────────────────────────────────────────────────┐
│                       OpenMindAI Mobile                      │
├──────────────────────────────────────────────────────────────┤
│ Flutter / Dart                                              │
│ Chat · Model Manager · Downloads · Settings · Local context │
├──────────────────────────────────────────────────────────────┤
│ Device profile + model router + local storage               │
├──────────────────────────────────────────────────────────────┤
│ lib_llama_cpp / llama.cpp                                   │
├──────────────────────────────────────────────────────────────┤
│ Local GGUF model files                                      │
└──────────────────────────────────────────────────────────────┘
```

### Main technologies

| Area | Technology |
| --- | --- |
| Desktop shell | Tauri 2 |
| Desktop backend | Rust 2021 |
| Desktop frontend | React 18 + TypeScript + Vite 6 |
| Mobile | Flutter + Dart |
| Mobile local LLM bridge | `lib_llama_cpp` / llama.cpp |
| Local database | SQLite |
| Networking/downloads | Reqwest on desktop; mobile-native Flutter networking stack |
| Local model format | GGUF for supported LLM families |
| Updates | Tauri updater on desktop; mobile release packaging remains preview-stage |
| CI | GitHub Actions |
| Security scanning | CodeQL, `npm audit`, `cargo audit` |

---

## First Run

### Desktop

1. Install or launch OpenMindAI.
2. Choose where OpenMindAI AI data should live.
3. Create the local profile used by the application.
4. OpenMindAI detects available hardware.
5. The required local runtime is prepared.
6. Select or download a supported model.
7. The model is verified before it is treated as ready.
8. OpenMindAI opens into the desktop workspace.

### Mobile preview

1. Install the mobile preview build.
2. OpenMindAI reads the mobile device profile.
3. Onboarding recommends models that fit the detected RAM/storage/architecture.
4. Select a local model.
5. Watch real download progress and speed.
6. The model is validated before installation is considered complete.
7. Chat runs locally through the bundled llama.cpp runtime.

Internet access is needed for initial runtime/model downloads and features that explicitly use online services. Once the required local components are installed, local inference can continue offline.

---

## Storage and portability

OpenMindAI Desktop separates the application from the user's AI data. The selected storage root, referred to internally as `OPENMINDAI_ROOT`, can contain:

- model files;
- AI runtimes;
- SQLite database;
- cache;
- logs;
- generated artifacts;
- workspaces;
- knowledge data;
- backups.

When an `openmindai.marker` file is present at the portable root, OpenMindAI can resolve its data location relative to that root, making external-drive workflows more reliable.

Mobile models use app-private device storage rather than the desktop portable-root system.

---

## Default local models

The stable desktop `v3.0.0` baseline targets **Qwen3 4B (`Q4_K_M`)** as the default local language model.

The mobile preview starts from smaller hardware-aware options such as **OpenMindAI Nano (Qwen3 0.6B)** and then exposes larger compatible models in Model Manager according to device capability.

Model weights are deliberately excluded from this public repository. They are downloaded separately and remain subject to upstream model licenses.

Actual inference performance depends on CPU, GPU/NPU support where applicable, available RAM, model size, quantization, context length, thermal limits, and runtime backend support.

---

## Offline and online behavior

OpenMindAI is local-first, not network-blind.

### Designed to work offline after local setup

- local AI inference;
- normal chat and conversation history;
- local settings and profile data;
- installed model/runtime management;
- desktop Projects and attached local workspaces;
- desktop Project Agent file operations;
- local diagnostics and maintenance;
- supported local document/media workflows.

### Requires network access when used

- initial bootstrap/source downloads;
- runtime/model downloads;
- application and model update checks;
- Web Search and Deep Research retrieval;
- connected external services;
- mobile preview binary distribution/update checks when enabled.

A temporary inability to reach GitHub or another external provider should not make an already-prepared local AI environment unusable for its offline capabilities.

---

## Development

### Desktop requirements

- **Node.js 24**
- npm
- a current stable Rust toolchain
- platform-specific Tauri build dependencies

### Run desktop locally

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

The desktop application version is synchronized across `package.json`, the Rust crate, and Tauri configuration. The unofficial mobile preview label is documentation/release metadata and does not change those stable desktop version files.

---

## Continuous Integration

Desktop CI validates frontend build quality, Rust formatting/lint/tests, release consistency, and security workflows.

The separately maintained mobile CI validates:

- Dart formatting;
- Flutter static analysis;
- Flutter tests;
- Android debug APK build;
- bundled llama runtime coverage for `armeabi-v7a`, `arm64-v8a`, and `x86_64`.

---

## Security

OpenMindAI gives local AI meaningful access to files, optional terminal commands, local model runtimes, and optional connected services, so permission boundaries are part of the product.

Key desktop controls include:

- explicit Tauri capability configuration;
- restrictive Content Security Policy;
- scoped project folder access by default;
- explicit approval for Full PC + Terminal access;
- approval guards for direct workspace mutations and remote mutating actions;
- bounded agent steps and terminal execution;
- secret/token storage outside conversation history;
- external provider data treated as untrusted input;
- local model/runtime binaries excluded from source control;
- `npm audit`, `cargo audit`, and CodeQL security checks.

Mobile security focuses on app-private model storage, local inference, explicit compatibility checks, and avoiding hidden fallback to paid cloud inference APIs for supported local model flows.

See [`SECURITY.md`](SECURITY.md) for desktop security policy.

---

## Release pipeline

### Stable desktop release

Tagged desktop releases use [`.github/workflows/release.yml`](.github/workflows/release.yml). The release path checks version consistency, application builds, installer output, updater configuration, release metadata, and other release-readiness requirements.

### Unofficial mobile preview

`v3.1.0-unofficial.1` is a **preview identifier**, not the new stable desktop application version.

If a binary is published under this label:

- mark it as a **GitHub pre-release**;
- clearly label it **Unofficial Mobile Preview**;
- do not replace the stable `v3.0.0` desktop installer links;
- do not describe it as production-ready Android/iOS distribution;
- note that device/model compatibility varies by RAM, storage, CPU architecture, and model size;
- preserve upstream model licensing notices.

Release notes are prepared in [`.github/releases/v3.1.0-unofficial.1.txt`](.github/releases/v3.1.0-unofficial.1.txt).

---

## Platform notes

### Windows

Windows x64 is the primary packaged and tested desktop platform for `v3.0.0`. The release uses a Tauri NSIS installer.

### Linux

The Linux bootstrap supports the source/bootstrap installation path. Desktop environments and GPU stacks vary, so hardware-specific validation continues to expand.

### macOS

The macOS bootstrap supports Intel/Apple Silicon source/bootstrap environments. Distribution signing and notarization remain separate from source-level compatibility.

### Android preview

The current Android mobile runtime supports `armeabi-v7a`, `arm64-v8a`, and `x86_64`. Model compatibility is still constrained by available RAM and storage.

### iOS preview

The Flutter mobile application includes an iOS target, but the `v3.1.0-unofficial.1` label remains a development preview rather than a production App Store release.

---

## Troubleshooting

### Setup cannot download a model

Check network access and available storage, then retry. Interrupted downloads are designed to resume instead of forcing a full restart where supported.

### A mobile model is visible but cannot be installed

The model may exceed the device RAM/storage requirement, may require a 64-bit runtime, or may be intentionally marked manual/not-recommended because of size or upstream licensing terms.

### Local inference cannot start on Android

Confirm the installed APK matches a supported ABI and that the selected model is fully installed. Current Android builds cover `armeabi-v7a`, `arm64-v8a`, and `x86_64`.

### The configured desktop storage drive is missing

Reconnect the drive or restore the configured location. OpenMindAI should not silently create a second unrelated data root.

### GPU acceleration is unavailable

Use a compatible fallback backend where available. Local CPU inference is slower but remains the broad desktop compatibility path for supported models.

### Project Agent cannot run terminal commands

Terminal execution is intentionally disabled until **Full PC + Terminal** access is explicitly enabled for the project.

---

## Repository policy

This public repository contains the **OpenMindAI Desktop** source code, build configuration, bootstrap tooling, documentation, workflow configuration, and small project assets.

The separately maintained mobile application source is not included in this public repository.

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

---

## Contributing

OpenMindAI is under active development. Useful contributions include reproducible bug reports, platform validation, performance measurements, security improvements, documentation fixes, connector improvements, and focused desktop code changes.

Before opening a desktop pull request, run the same core checks used by CI:

```bash
npm ci
npm run check:version
npm run lint
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-features
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for contribution guidance.

---

## License

OpenMindAI Desktop source code in this repository is released under the [Apache License 2.0](LICENSE).

Third-party components and attribution notices are documented in [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt).

Downloaded AI models are governed by their respective upstream licenses and are not relicensed by this repository.

---

## Maintainer

**Md Shahanur Islam Shagor**  
Full-Stack Web Developer & AI Engineer

- GitHub: [@smshagor-dev](https://github.com/smshagor-dev)
- Portfolio: [smshagor.com](https://smshagor.com)

---

If OpenMindAI is useful to you, consider starring the repository. It helps other developers find the project and supports continued work on practical, user-controlled local AI software.
