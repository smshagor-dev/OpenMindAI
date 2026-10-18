# ZIP 7.2.0 -> ZIP 9.0.4: upstream compatibility and removal plan

**Status: NOT REMEDIATED.** This is an executable migration experiment, not a
production patch. Issue #258 stays open and the three scoped RustSec exceptions
remain necessary until all affected packages have been replaced.

## Why a Cargo.toml-only upgrade cannot work

The current pinned `any-tts 0.1.3` and `candle-core 0.10.2` crates each
require ZIP 7.x. Cargo supports simultaneous major versions, so adding a direct
`zip = "9.0.4"` dependency does **not** remove their ZIP 7 dependency.
Likewise, manually deleting the ZIP 7 entry from Cargo.lock or removing
`cargo audit --ignore` without migrating the consumers does not fix the code.

The tested Candle ZIP 9 migration PRs have not been incorporated into a
compatible published 0.10.x release. Do **not** blindly replace the entire
Candle 0.10.x family with Candle 0.11: that is a separate API/binary-compatibility
migration and the published 0.11.0 still uses legacy ZIP 8.

## Reproducible upstream experiment

Run `python3 scripts/probe-zip9-upgrade.py --self-test` to check the
fail-closed Cargo manifest transformation. The GitHub Actions workflow
`ZIP 9 Upstream Compatibility` then:

1. Fetches the exact Cargo.lock-pinned `any-tts@0.1.3` and
   `candle-core@0.10.2` packages (Cargo verifies their registry checksums).
2. Copies their sources to an **ephemeral directory**, changing only their ZIP
   version requirements to patched ZIP 9.0.4.
3. Uses Cargo's temporary `[patch.crates-io]` CLI overrides and re-resolves an
   ephemeral lockfile; fails if **any ZIP 7 or ZIP 8** remains.
4. Compiles the OpenMindAI desktop Rust library with these substitutes to
   expose any ZIP 9 API errors. It restores the original production lockfile
   even if a step fails.

A red result is **evidence of a migration blocker**, not permission to
disable the actual security gate. The probe does not test full speech output,
ZIP member semantics, Windows/macOS runtime behavior, or package provenance.

## Conditions for the real fix

- Adapt all ZIP 9 API changes in **reviewed**, license-compliant upstream forks
  or vendor snapshots of exact `any-tts 0.1.3` and
  `candle-core 0.10.2` sources. Include provenance, file checksums and
  explicit diff for each patch. If available, prefer new published upstream
  releases that themselves use patched ZIP 9.
- Perform Candle NPZ and PyTorch `.pth` / `.pt` loader regression tests on
  Windows, macOS and Linux using real representative inputs. Confirm ZIP
  filename/path behavior, symlink and Unix permission safety, and Kokoro
  output. A successful `cargo check` is **not** equivalent to these tests.
- Update the actual `src-tauri/Cargo.toml` and regenerate
  `src-tauri/Cargo.lock` using pinned, auditable crate sources.
- Assert that ZIP 7/8 disappear from the entire lockfile, then remove
  `--ignore RUSTSEC-2026-0367/0368/0369` and the expiring scope guard.
- Run **unignored** `cargo audit --file src-tauri/Cargo.lock`, full TTS
  regressions, and all desktop CI/security/release-readiness jobs.
- Close issue #258 only after all of the above pass without exceptions.

**Deadline:** existing scoped exceptions expire **2026-11-17**. A passing
current Security workflow with these exceptions is not vulnerability
remediation.
