# OpenMindAI Mobile App Plan

## Direction

OpenMindAI Mobile will be developed from the existing Tauri 2 + React + TypeScript + Rust codebase instead of starting a separate Flutter or React Native product. The first target is Android, followed by iOS once the shared mobile layer and Android runtime are stable.

The goal is one OpenMindAI ecosystem with a shared product language and shared core logic, while keeping platform-specific capabilities explicit.

```text
OpenMindAI
├── Desktop
│   ├── Windows
│   ├── Linux
│   └── macOS
├── Mobile
│   ├── Android   <- first target
│   └── iOS       <- second target
└── Shared product core
    ├── React + TypeScript UI
    ├── conversations and projects
    ├── connected apps
    ├── artifact flows
    ├── settings and preferences
    └── Rust services that are safe on the target platform
```

## Product principles

1. **Local-first where practical.** Chat history, settings, project metadata, and supported inference data remain device-local by default.
2. **No fake desktop access on mobile.** Android and iOS sandbox restrictions are treated as a product boundary, not hidden behind an unreliable terminal abstraction.
3. **Permission before power.** File, media, notification, connected-service, and future remote-agent permissions remain explicit.
4. **Shared UX, native behavior.** The product should feel like OpenMindAI on every device while respecting mobile navigation, touch targets, safe areas, background limits, and app lifecycle rules.
5. **Desktop remains stable.** Mobile changes must not regress Windows, Linux, or macOS behavior.

## Mobile information architecture

### Primary surfaces

- **Chat** — local/connected AI conversations, attachments, vision, search, research, voice.
- **Work** — mobile-safe AI work and future remote Desktop Agent sessions.
- **Projects** — instructions, linked chats, reference files, repository/service context.
- **Library** — generated documents, images, audio, and other artifacts.
- **More** — Models, Connected Apps, Tools, Settings, diagnostics, storage.

### Navigation design

Mobile uses a chat-first layout:

- compact top bar;
- Chat / Work switch remains available;
- sidebar becomes a slide-in navigation/history drawer;
- persistent bottom tab bar exposes high-frequency areas;
- desktop title-bar controls and sidebar resizing are removed from the mobile presentation;
- all interactive controls target touch-friendly sizing;
- bottom and top safe-area insets are respected.

## Feature matrix

| Capability | Desktop | Android target | iOS target | Notes |
| --- | --- | --- | --- | --- |
| Chat/history | Yes | Phase 1 | Phase 4 | Shared UI and local data |
| Thinking mode | Yes | Phase 1 | Phase 4 | Subject to mobile model capability |
| Image/file attachments | Yes | Phase 1 | Phase 4 | Uses mobile-safe file/media selection |
| Vision | Yes | Phase 2 | Phase 4 | Device/model dependent |
| Web Search / Research | Yes | Phase 1 | Phase 4 | Requires network |
| Projects | Yes | Phase 1 | Phase 4 | Mobile-safe project context |
| Full local Project Agent | Yes | Limited | Limited | No unrestricted mobile shell |
| Full PC + Terminal | Yes | No | No | Desktop-only security boundary |
| Remote Desktop Agent | Planned | Phase 3 | Phase 4 | Phone controls trusted OpenMindAI Desktop |
| Connected Apps | Yes | Phase 2 | Phase 4 | OAuth/deep-link work required |
| Local lightweight models | Yes | Phase 2 | Phase 4 | Hardware/RAM tiered |
| Large desktop GGUF models | Yes | Device dependent | Device dependent | Not a default mobile assumption |
| Voice input/output | Yes | Phase 2 | Phase 4 | Native permissions/lifecycle |
| Image generation | Yes | Device dependent | Device dependent | Runtime/model dependent |
| App updater | Yes | Platform policy | Platform policy | Distribution rules differ by store |

## Model strategy

The desktop default must not be blindly reused on every phone. Mobile setup will recommend a model tier from detected device capability.

| Device class | Initial target |
| --- | --- |
| 4-6 GB RAM | 0.5B-1.5B quantized model |
| 6-8 GB RAM | 1.5B-3B quantized model |
| 8-12+ GB RAM | 3B-4B where thermal/memory tests pass |
| High-end devices | Optional larger model after benchmark |

Mobile model selection must consider available memory, architecture, thermal behavior, storage, context length, and measured inference performance rather than RAM alone.

## Project Agent design on mobile

### Mobile Local Agent

Allowed direction:

- read files selected or stored inside the application's permitted workspace;
- edit mobile-safe project files;
- analyze documents and source code;
- work with GitHub through the connected GitHub capability;
- produce patches, files, plans, and validation guidance.

Not treated as equivalent to desktop:

- unrestricted host filesystem;
- arbitrary Full PC access;
- desktop shell/PowerShell behavior;
- silent package installation or privileged commands.

### Remote Desktop Agent

Phase 3 introduces the high-power mobile workflow:

```text
OpenMindAI Mobile
        |
        | authenticated encrypted session
        v
Trusted OpenMindAI Desktop
        |
        +-- local models
        +-- Project Agent
        +-- project files
        +-- Git
        +-- terminal
        +-- tests/build/lint
```

The phone becomes the secure control surface while execution remains on the user's trusted desktop. Remote execution must preserve the existing Full PC + Terminal approval model and expose action/validation status to mobile.

## Delivery phases

### Phase 0 - Mobile foundation (current)

- add Android/iOS Tauri CLI scripts;
- separate desktop and mobile Tauri capabilities;
- introduce responsive mobile shell styles;
- convert sidebar behavior to a mobile drawer;
- add touch-friendly bottom navigation;
- respect Android/iOS safe areas;
- hide desktop-only window and resize controls on narrow/mobile layouts;
- document platform boundaries and build workflow;
- validate existing frontend/desktop CI through a pull request.

### Phase 1 - Android application shell

- run `tauri android init` and commit the generated Android project where appropriate;
- establish Android application ID, icons, splash screen, signing strategy, and build variants;
- run on emulator and a physical ARM64 Android device;
- make setup, chat, history, composer, Projects, Library, Tools, and Settings touch-ready;
- implement mobile file/media selection behavior;
- audit every Tauri command for Android compatibility;
- gate desktop-only commands at both UI and Rust layers;
- add Android CI build validation.

Exit criteria:

- debug APK launches on emulator and physical device;
- user can complete mobile-safe setup;
- user can create/open conversations and persist history;
- no desktop-only control is exposed as if it worked on Android;
- desktop CI remains green.

### Phase 2 - Local mobile AI + connected features

- benchmark ARM64 local inference backends;
- implement device capability detection and mobile model tiers;
- model download/storage lifecycle for Android;
- vision attachment path;
- voice input/output;
- Web Search and Deep Research UX;
- Google Workspace and GitHub mobile OAuth/deep-link validation;
- expand to Microsoft 365, Slack, Notion, Dropbox, and compatible MCP workflows.

Exit criteria:

- supported devices can download and run at least one recommended local model;
- memory/thermal failure states are handled cleanly;
- connected-service tokens remain outside conversation history;
- network-only features fail gracefully offline.

### Phase 3 - Desktop sync and Remote Project Agent

- trusted-device pairing;
- encrypted session transport;
- desktop discovery/pairing UX;
- remote chat/history synchronization policy;
- remote Project Agent command stream;
- permission prompts and action audit trail;
- reconnect/resume behavior;
- remote cancellation;
- validation evidence returned to mobile.

Exit criteria:

- mobile can securely instruct a paired desktop Project Agent;
- terminal/file execution never occurs on an unpaired machine;
- existing desktop approval boundaries remain enforceable;
- session can be revoked from either device.

### Phase 4 - iOS

- run `tauri ios init` on macOS with Xcode;
- apply shared mobile UI and capability layer;
- adapt storage/document-picker behavior;
- validate local inference options on Apple mobile hardware;
- configure signing, provisioning, TestFlight, privacy manifests, and App Store requirements;
- port trusted Desktop Agent pairing.

## Repository layout target

```text
src/
├── components/
│   └── mobile-aware shared UI
├── lib/
│   └── platform capability helpers
├── mobile.css
└── ...shared frontend

src-tauri/
├── capabilities/
│   ├── default.json     # desktop targets
│   └── mobile.json      # Android/iOS targets
├── gen/
│   ├── android/         # generated after android init
│   └── apple/           # generated during iOS phase
├── src/
│   └── shared + platform-gated Rust services
└── tauri.conf.json
```

## Android developer workflow

Prerequisites include the Tauri requirements plus Android SDK/Android Studio, an Android NDK supported by the current Tauri toolchain, Java, and the required Rust Android targets.

```bash
npm ci
npm run android:init
npm run android:dev
```

Build APK/AAB artifacts:

```bash
npm run android:build
```

Run a production-mode Android build on a connected device:

```bash
npm run android:run
```

The generated Android project should not be manually invented. It is created by the Tauri CLI so its Gradle/Kotlin configuration matches the installed Tauri version.

## iOS developer workflow

Final iOS development requires macOS and Xcode.

```bash
npm ci
npm run ios:init
npm run ios:dev
npm run ios:build
```

## Security checklist

Before mobile release:

- platform-specific Tauri capabilities are minimal and reviewed;
- desktop-only terminal/full-PC commands are not exposed on mobile;
- OAuth redirects use supported mobile deep-link/app-link flows;
- secrets stay in the platform credential/keychain layer;
- file access is scoped to explicit app/user grants;
- remote desktop pairing uses authenticated encryption and revocable device identity;
- logs do not contain access tokens, model prompts marked private, or sensitive file contents;
- Android exported activities/services/providers are reviewed;
- release signing keys are not stored in the repository;
- dependency and mobile build scans run in CI.

## First release target

The first public mobile release should be an **Android beta**, not a feature-parity promise with desktop. It should focus on a reliable chat-first OpenMindAI experience, mobile-safe Projects, connected features, and a clear upgrade path toward local mobile models and the Remote Desktop Agent.
