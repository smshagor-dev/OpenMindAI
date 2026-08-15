# Release Process

## Versioning

OpenMindAI uses semantic versioning (`MAJOR.MINOR.PATCH`). The version is
kept in exactly three places, which must always agree: `package.json`,
`src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`. Bump all three
together with:

```powershell
scripts\bump-version.ps1 -Version 1.2.3
```

This edits all three files atomically (it validates every file has exactly
one version field before changing any of them) and does nothing else — it
doesn't tag, commit, or publish anything.

Application releases and the local database's schema version are
independent — see [docs/DATABASE.md](DATABASE.md). A `1.1.0` app release
doesn't imply a schema version bump, and vice versa.

## Building a release installer

```powershell
scripts\build-installer.ps1
```

This runs the production frontend build, compiles the Rust backend in
release mode (LTO, single codegen unit, stripped symbols), bundles it as a
Windows NSIS installer, and writes a `SHA256SUMS.txt` next to it. Output
location depends on your Cargo target directory (the default is
`src-tauri\target\release\bundle\nsis\`; this machine's dev setup redirects
it elsewhere via `src-tauri\.cargo\config.toml` — the script resolves the
actual path via `cargo metadata` rather than assuming the default).

The installer:

- Installs per-user (no administrator prompt required).
- Adds a Start Menu shortcut.
- Uninstalling from Windows Settings → Apps removes only the application
  files — it never touches `OPENMINDAI_ROOT` (your models, database,
  conversations), since the two live in entirely separate locations by
  design. See [docs/PORTABLE-STORAGE.md](PORTABLE-STORAGE.md).

## Current release-readiness gaps

Being direct about what isn't finished yet, rather than implying it is:

- **Code signing.** No Windows code-signing certificate is configured.
  Installers ship unsigned — Windows SmartScreen will warn users on first
  run (see [docs/TROUBLESHOOTING.md](TROUBLESHOOTING.md)). Verify a
  download against its `SHA256SUMS.txt` instead of relying on a publisher
  signature until this changes. Once a certificate is available, this is a
  small `bundle.windows` addition to `tauri.conf.json` plus a signing step
  in `scripts\build-installer.ps1` — not an architectural change.
- **Application update hosting.** `tauri.conf.json`'s updater `endpoints`
  still points at a placeholder
  (`https://REPLACE-BEFORE-RELEASE.invalid/...`). The client-side
  download/verify/install pipeline is fully built and was tested against a
  local mock manifest (see [docs/UPDATES.md](UPDATES.md)) — what's missing
  is standing up real hosting for the update manifest JSON, most likely via
  GitHub Releases (the conventional path for Tauri's updater), which in
  turn depends on this repository having real git/GitHub infrastructure set
  up, which it does not yet.
- **Git repository / GitHub release.** This repository is not yet under
  git version control. Setting that up, plus preparing an actual GitHub
  release (tag, release notes, uploaded installer + checksums), is a
  prerequisite for both public distribution and the update-hosting item
  above, and hasn't been done yet.

None of these block building and running the installer locally — they block
*public, signed, auto-updating* distribution specifically.

## Release checklist

- [ ] `scripts\bump-version.ps1` run, all three version fields agree.
- [ ] `cargo test`, `cargo clippy`, `npm run lint`, `npm run build` all pass.
- [ ] `scripts\build-installer.ps1` produces a working installer.
- [ ] Fresh-machine install test (no Node/npm/Rust/Cargo present) — see
  [docs/INSTALLATION.md](INSTALLATION.md).
- [ ] Second launch skips setup; offline launch still opens and chats.
- [ ] Uninstall preserves `OPENMINDAI_ROOT`; reinstall picks the existing
  data back up.
- [ ] `CHANGELOG.md` updated for this version.
- [ ] Known gaps above are still accurately described (or resolved and this
  file is updated to say so).
