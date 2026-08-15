# Changelog

All notable changes to OpenMindAI are documented here. Format loosely
follows [Keep a Changelog](https://keepachangelog.com/); versioning is
[semantic](https://semver.org/).

## [Unreleased]

Production-hardening pass toward a v1.0.0 release — turning the working
development build into installable, self-updating consumer software. No new
AI capabilities in this pass; see [docs/ROADMAP.md](docs/ROADMAP.md) for
what comes after v1.0.0.

### Added

- Production release profile (LTO, stripped symbols) and a
  `scripts\bump-version.ps1` helper that keeps `package.json`,
  `tauri.conf.json`, and `Cargo.toml` version fields in sync.
- Windows NSIS installer (`scripts\build-installer.ps1`), per-user install
  (no admin prompt), branded icons, `SHA256SUMS.txt` generation.
- Setup wizard resumability from any step (not just the download stage) via
  an explicit persisted setup-state machine.
- A real application-update client pipeline: download, verify, install,
  and restart — not just a check — plus Windows notifications for both
  application and model update availability.
- Structured application logging (`tracing`, rotated daily into
  `OPENMINDAI_ROOT\logs\`) covering startup, setup, runtime/model install,
  maintenance, and update-check events — never conversation content.
- An in-app log viewer in the Maintenance Center.
- A real numbered database migration runner (tracked, one-time-applied,
  transactional) replacing the previous idempotent-replay-on-every-open
  approach.
- On-boot integrity re-verification for an already-downloaded AI model, so
  a corrupted file gets re-downloaded instead of silently trusted.
- `LICENSE` (Apache-2.0) and `THIRD_PARTY_NOTICES.md`.
- User-facing documentation: `docs/INSTALLATION.md`,
  `docs/OFFLINE-MODE.md`, `docs/TROUBLESHOOTING.md`,
  `docs/MODEL-MANAGEMENT.md`, `docs/UPDATES.md`, `docs/MAINTENANCE.md`,
  `docs/RELEASE.md`.

### Changed

- Application files and user data (`OPENMINDAI_ROOT`) are now architecturally
  separate, matching a normal Windows install (previously the portable root
  layout included an unused `app/` folder suggesting otherwise).
- The model catalog and the model downloader now share one source of truth
  for which Hugging Face repo to fetch from, instead of duplicating it.

### Fixed

- `PortableRootManager::info()` always reported the default (non-profile)
  database path, even for installs using a custom profile name — latent
  since nothing displayed it yet, but incorrect.
- `HardwareProfile.backends.sycl`/`.hip` were hardcoded `false` even on
  machines with Intel/AMD GPUs, which the Settings backend-label display
  actually reads — understating real hardware capability.
- A test-isolation race between two Rust test modules that both touch the
  `OPENMINDAI_ROOT` environment variable under parallel test execution.
- `scripts\setup-msvc-env.ps1` failed when `vcvars64.bat`'s own internal
  `vswhere.exe` call couldn't find `vswhere` on `PATH`.

### Known gaps

Tracked honestly in [docs/RELEASE.md](docs/RELEASE.md): the installer is
unsigned (no code-signing certificate yet), the real application-update
hosting endpoint isn't live yet (client pipeline is built and tested against
a local mock manifest), and this repository isn't under git version control
yet.
