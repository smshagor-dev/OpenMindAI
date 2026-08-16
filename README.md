# OpenMindAI

OpenMindAI is a local-first, offline-first AI desktop application. It
downloads and runs an open-weight language model directly on your computer
— no cloud account, no API key, no subscription. After first-run setup, it
works fully offline.

Official repository: https://github.com/smshagor-dev/OpenMindAI

## Quick Download

| Platform | File | Status |
| --- | --- | --- |
| Windows — Recommended | [OpenMindAI-Setup-v1.0.1-x64.exe](https://raw.githubusercontent.com/smshagor-dev/OpenMindAI/main/OpenMindAI_1.0.1_x64-setup.exe) | Tested / Stable |
| Windows — Git Bootstrap | [`OpenMindAI-Setup.bat`](https://raw.githubusercontent.com/smshagor-dev/OpenMindAI/main/OpenMindAI-Setup.bat) | Tested / Stable |
| Linux | [`openmindai-setup.sh`](https://raw.githubusercontent.com/smshagor-dev/OpenMindAI/main/openmindai-setup.sh) | Tested / Stable |
| macOS | [`OpenMindAI-Setup.command`](https://raw.githubusercontent.com/smshagor-dev/OpenMindAI/main/OpenMindAI-Setup.command) | Tested / Stable |

The Windows `.exe` installer is the simplest path once it's published as
part of the v1.0.1 GitHub Release. The bootstrap scripts (`.bat`/`.sh`/
`.command`) work today — they clone the official source and build/launch
OpenMindAI directly, and will automatically switch to downloading a
prebuilt release once one is published. See
[Support Matrix](#support-matrix) below for validation status.

## Windows Installation

1. Download `OpenMindAI-Setup-v1.0.1-x64.exe`.
2. Run it. It's currently **unsigned** (no code-signing certificate yet),
   so Windows SmartScreen will warn — click "More info" → "Run anyway" if
   you trust the source, or verify it against the published
   `SHA256SUMS.txt` first.
3. Choose your AI storage location and a local profile name.
4. OpenMindAI configures its local database, detects your hardware,
   downloads the matching AI engine, then downloads and verifies the AI
   model (Qwen3 4B).
5. OpenMindAI opens. Chat now works fully offline.

## Windows Git Bootstrap

1. Download `OpenMindAI-Setup.bat`.
2. Put it in the folder/drive you want OpenMindAI's source and build in —
   any drive, including a USB or external SSD, with or without spaces in
   the path.
3. Double-click it.
4. It checks for Git (installing it via `winget` if missing and available),
   then clones the official repository:
   `https://github.com/smshagor-dev/OpenMindAI.git`
5. It looks for a compatible prebuilt release; if none exists yet, it
   builds OpenMindAI from source automatically (requires Node.js and
   Rust/Cargo — it tells you clearly if either is missing, with where to
   get it, rather than failing silently).
6. OpenMindAI launches and first-run setup begins.
7. Running the `.bat` again later reuses the existing installation — it
   does not re-clone, re-install dependencies, or rebuild unless something
   actually changed.

You never need to type a Git command yourself.

## Linux Installation

```sh
chmod +x openmindai-setup.sh
./openmindai-setup.sh
```

It detects your package manager (`apt`, `dnf`, `yum`, `pacman`, `zypper`,
or `apk`), installs Git if missing (asking for `sudo` only when actually
needed), clones the official repository, and either downloads a compatible
prebuilt release or builds from source. x86_64 is the only architecture
currently targeted — ARM64 is not yet built or tested. This script has been
written and reviewed but not yet run-tested on a real Linux machine; treat
it as beta until that's done.

## macOS Installation

Double-click `OpenMindAI-Setup.command`, or from Terminal:

```sh
chmod +x OpenMindAI-Setup.command
./OpenMindAI-Setup.command
```

Detects Intel vs. Apple Silicon automatically. Uses Apple's own Xcode
Command Line Tools for Git if it's missing (or Homebrew, if you already
have it installed — OpenMindAI never installs Homebrew for you without
asking). A downloaded/built OpenMindAI app is not currently signed or
notarized, so Gatekeeper will warn on first open; you'll need to allow it
via System Settings → Privacy & Security. This script has been written and
reviewed but not yet run-tested on a real Mac; treat it as experimental
until that's done.

## First Launch

Every platform follows the same setup: choose AI storage → create a local
profile → OpenMindAI detects your hardware → downloads the AI engine →
downloads and verifies the AI model (Qwen3 4B, `Q4_K_M`) → ready to chat.

## Portable Mode

If OpenMindAI (or one of the bootstrap scripts) finds an `openmindai.marker`
file near its own location, it treats that folder as its data root instead
of asking where to store data. This makes the whole install portable —
models, runtime, and database move with the folder. If a portable install
moves to a different drive letter (e.g. `G:\OpenMindAI` becomes
`H:\OpenMindAI` after being plugged into a different port), it keeps
working: paths are resolved relative to the marker's location, not a fixed
drive letter.

## Offline Mode

**Needs internet:** first-run setup (downloading the AI engine and model),
the Git bootstrap's source clone/update step, and update checks.

**Works fully offline:** chatting, conversation history, local settings,
GPU/CPU inference, and the Maintenance Center's diagnostics/repair/backup
tools — once setup has completed once.

If a bootstrap script or OpenMindAI itself can't reach the internet on a
later run, it skips the update/clone step entirely and launches your
existing installation immediately. It never refuses to start just because
GitHub or an update server is unreachable.

## Updates

Application updates and model updates are independent, and both are
notify-first, not silent:

- **Application updates** — OpenMindAI checks for a newer release in the
  background and lets you download, verify, and install it from inside the
  app, with an explicit restart step you control.
- **Model updates** — checked separately; large model downloads are never
  started automatically unless you opt in.

The bootstrap scripts (`.bat`/`.sh`/`.command`) intentionally don't
implement their own separate update-checking logic — they only handle
getting OpenMindAI installed and launched. All update checking happens
inside the app itself, so there's one update system, not two competing
ones.

## Storage

All of OpenMindAI's data — models, runtime, database, chat history, cache,
generated files, workspaces, knowledge, backups, logs — lives under a
folder you choose during setup, referred to as `OPENMINDAI_ROOT`. This is
kept separate from wherever the application itself or its source code is
installed.

## Model

OpenMindAI ships against **Qwen3 4B (`Q4_K_M`)** by default. Model weights
are never stored in this Git repository — they're downloaded and
checksum-verified during setup.

## Support Matrix

| Platform | Status |
| --- | --- |
| Windows x64 | Tested / Stable |
| Linux x64 | Implemented / Not yet run-tested on real hardware |
| macOS (Intel / Apple Silicon) | Implemented / Not yet run-tested on real hardware |
| Linux ARM64 | Not implemented |

## Troubleshooting

- **Git not found / can't install it automatically** — install it yourself
  (Windows: https://git-scm.com/download/win · Linux: your distro's package
  manager · macOS: `xcode-select --install`), then run setup again.
- **No internet on first run** — setup can't complete without it (the AI
  engine and model have to be downloaded once). Connect and try again.
- **Model download interrupted** — resumes automatically from where it left
  off; no need to start over.
- **No compatible AI runtime / GPU not detected** — OpenMindAI falls back
  automatically (GPU backend → Vulkan → CPU); chat still works, just slower
  without GPU acceleration.
- **Storage location unreachable** (e.g. an external drive unplugged) —
  OpenMindAI won't silently create a new, different storage location; it
  tells you the original one is missing so you don't lose track of your
  data.
- **Windows SmartScreen warning** — expected for the unsigned installer;
  verify against `SHA256SUMS.txt` instead.

## Build From Source

For developers only — normal users should use the downloads above.

```sh
git clone https://github.com/smshagor-dev/OpenMindAI.git
cd OpenMindAI
npm install
npm run tauri dev
```

Requires Node.js/npm and Rust/Cargo (Windows also needs MSVC Build Tools).
Production build: `npm run build && npm run tauri -- build`.

## License

[Apache License 2.0](LICENSE). Third-party attributions:
[THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt).
