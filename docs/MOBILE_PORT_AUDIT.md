# OpenMindAI Mobile Port Audit

This audit records the first code-level blockers and compatibility decisions for the Android-first port. It is intentionally separate from the product roadmap so each item can be converted into an implementation task and validation gate.

## Current foundation

- Tauri 2 desktop application.
- React 18 + TypeScript + Vite frontend.
- Rust application core.
- SQLite/local data model.
- Local model/runtime management.
- Projects and Project Agent.
- Connected apps.
- Artifact/media workflows.

## Compatibility findings

### 1. Tauri mobile entry point

Current Rust application startup exposes `pub fn run()` without the mobile entry annotation used by Tauri mobile applications.

Phase 1 action:

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // existing builder
}
```

This must be applied together with an Android compile check rather than as an isolated cosmetic change.

### 2. Desktop and mobile capabilities must be separated

The previous default capability included desktop window controls for every target. The mobile foundation splits capabilities by platform:

- `default.json` -> Linux, macOS, Windows;
- `mobile.json` -> Android, iOS.

Future mobile permissions should be added to `mobile.json` only when the feature is implemented and reviewed.

### 3. Desktop window UX

Desktop UI currently contains:

- resizable fixed sidebar;
- minimize/maximize/close controls;
- desktop title-bar drag behavior;
- absolute desktop status/footer presentation.

The mobile foundation changes presentation at the responsive boundary without removing desktop behavior:

- slide-in navigation/history drawer;
- touch bottom navigation;
- desktop window controls hidden;
- sidebar resize hidden;
- safe-area aware top/bottom layout;
- mobile-sized composer/messages/modals.

### 4. Portable root and filesystem assumptions

Desktop OpenMindAI supports user-controlled roots, external drives, local project folders, runtimes, models, generated files, logs, workspaces, knowledge data, and backups.

Android/iOS do not provide the same unrestricted filesystem model.

Phase 1 actions:

- define a mobile application root inside supported app storage;
- introduce user-selected/document-provider file access separately;
- prevent a missing desktop-style external root from forcing a broken first-run path;
- audit every command that opens/reveals a host filesystem path;
- preserve data separation between app bundle and user AI data where the platform permits it.

### 5. Local runtime assumptions

The current desktop runtime lifecycle expects locally managed AI runtime binaries and desktop launch behavior.

Phase 1/2 actions:

- identify Rust/runtime code that compiles unchanged on Android ARM64;
- gate desktop executable-launch behavior;
- benchmark a supported Android inference path;
- introduce mobile model tiers;
- keep large desktop models opt-in rather than mobile defaults;
- expose unsupported runtime states explicitly instead of silently falling back to cloud inference.

### 6. Project Agent and terminal

Desktop Project Agent can use explicitly approved local filesystem and terminal capabilities. Mobile must not advertise equivalent Full PC + Terminal access.

Phase 1 actions:

- hide/disable unrestricted terminal controls on mobile;
- gate terminal/full-PC Rust commands by target and permission;
- retain scoped project/document analysis;
- use GitHub/connected APIs for repository work where appropriate;
- reserve full terminal execution for the future trusted Remote Desktop Agent.

### 7. Connected apps

Connected apps remain strategically important on mobile, but desktop OAuth assumptions must be validated against Android/iOS lifecycle and redirect handling.

Phase 2 actions:

- implement app/deep-link redirect handling where required;
- validate Google Workspace and GitHub first;
- confirm secrets use a mobile-appropriate secure credential store;
- make token refresh resilient to app background/resume;
- keep provider tool output untrusted.

### 8. Updater and store distribution

Tauri exposes updater support across mobile targets, but Android/iOS store distribution has different product and policy requirements from desktop installer updates.

Phase 1 action:

- separate desktop updater UX from mobile store/release-channel policy before public distribution.

### 9. Background work and lifecycle

Desktop inference can assume a long-running foreground application more often than mobile can.

Phase 2 actions:

- handle pause/resume and process recreation;
- persist streaming state defensively;
- prevent corrupted download/model state after interruption;
- define background behavior for long model downloads;
- avoid pretending long local generation continues when the OS suspends execution.

### 10. Thermal, memory, and storage constraints

Mobile local inference needs resource policy in addition to raw compatibility.

Phase 2 actions:

- detect device architecture and usable memory;
- establish model recommendation tiers;
- add free-space checks before downloads;
- add thermal/performance test cases on physical devices;
- reduce context/model defaults where necessary;
- make low-resource failures recoverable.

## Phase 1 engineering checklist

- [ ] Initialize Android target with Tauri CLI.
- [ ] Add `cfg_attr(mobile, tauri::mobile_entry_point)` and compile Android Rust target.
- [ ] Establish mobile application storage root.
- [ ] Audit and gate desktop-only commands.
- [ ] Run the current setup flow on Android.
- [ ] Run Chat/History on Android with persistent local data.
- [ ] Validate the mobile drawer and bottom navigation on emulator and physical device.
- [ ] Adapt file/image attachment picker.
- [ ] Gate Full PC + Terminal.
- [ ] Add Android CI debug build.
- [ ] Verify desktop frontend/Rust/Windows build remains green.

## Definition of a valid Android foundation

The port is considered structurally ready for local-model work when an Android debug build can launch, complete its mobile-safe setup, persist conversations, navigate Chat/Work/Projects/Library/Settings, select files/media through platform-safe APIs, and never expose a desktop-only action as functional when it is not supported.
