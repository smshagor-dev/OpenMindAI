# Installation

This page is for people installing OpenMindAI as an end user. If you're
setting up a development environment instead, see
[docs/DEVELOPMENT.md](DEVELOPMENT.md).

## Requirements

- Windows 10 or 11, 64-bit.
- ~5 GB free disk space for the AI runtime and model (more if you install
  additional models later).
- Internet connection for first-run setup only (downloading the AI engine
  and model). Not required afterward for chatting.

No Node.js, npm, Rust, Cargo, or Git is required — those are only needed if
you're building OpenMindAI from source.

## Install

1. Download `OpenMindAI-Setup-x64.exe` from the release you were given.
2. Run it. **The installer is currently unsigned** (see
   [docs/RELEASE.md](RELEASE.md)), so Windows SmartScreen will show a
   warning the first time — click "More info" → "Run anyway" if you trust
   the source you downloaded it from. You can verify the download against
   the published `SHA256SUMS.txt` instead of relying on a publisher
   signature.
3. The installer runs per-user (no administrator prompt) and adds a Start
   Menu shortcut.
4. Launch OpenMindAI. On first launch you'll see the setup wizard:
   - **Welcome** — a short overview.
   - **Choose AI Storage** — pick where OpenMindAI keeps your models,
     database, and conversations (this becomes `OPENMINDAI_ROOT` — see
     [docs/PORTABLE-STORAGE.md](PORTABLE-STORAGE.md)). Defaults to
     `C:\OpenMindAI`; pick an external/secondary drive if you'd rather keep
     multi-gigabyte model files off your system drive.
   - **Create Your Local AI Profile** — a name for your local database. No
     account, no email, nothing leaves your computer.
   - **Preparing OpenMindAI** — the app detects your CPU/GPU, downloads and
     validates the right AI engine build for your hardware, then downloads
     and verifies the AI model (Qwen3 4B, ~2.5 GB). If setup is interrupted
     here, relaunching resumes the download rather than starting over.
   - **OpenMindAI is Ready** — click through and start chatting.

If the app is closed partway through setup (even before the storage/profile
steps finish), relaunching resumes from where you left off instead of
starting over from "Welcome".

## Uninstall

Uninstall from Windows Settings → Apps, like any other program. This removes
the application files only — your `OPENMINDAI_ROOT` folder (models,
database, conversations, logs) is untouched by design, since the installer
and your data live in separate locations (see
[docs/PORTABLE-STORAGE.md](PORTABLE-STORAGE.md)). Delete that folder
yourself if you want to remove your data too.

## Troubleshooting installation

See [docs/TROUBLESHOOTING.md](TROUBLESHOOTING.md).
