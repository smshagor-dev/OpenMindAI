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
3. Copies the desktop crate (excluding `target/`) into the disposable
   directory **before** Cargo resolution. Cargo's `update` command is run only
   against that copied manifest/lockfile, with temporary `[patch.crates-io]`
   CLI overrides; fails if **any ZIP 7 or ZIP 8** remains.
4. Compiles the copied desktop Rust library with these substitutes to expose
   ZIP 9 API errors. Build artifacts can reuse the existing Cargo target cache,
   but the checked-in `src-tauri/Cargo.toml` and `Cargo.lock` are never given
   to a writing Cargo command. This remains safe when interrupted by SIGKILL:
   it does not depend on a `finally` block restoring the production lockfile.

The isolated probe copies `scripts/zip9-reader-regressions.rs` into a
standalone temporary Cargo test harness linked to the patched Candle source.
The test suite validates:

- ZIP 9 NPZ eager and lazy read/write round-trips with a concrete `f32` tensor;
- Candle's PyTorch PTH ZIP filename handling and pickle parser on a small
  protocol-2 empty dictionary archive (`data.pkl`);
- loading a **nonempty** three-element `f32` tensor from the separate
  `archive/data/0` storage inside a PyTorch-format ZIP checkpoint, checking
  both parsed tensor metadata and the exact reconstructed values;
- rejecting a checkpoint that has valid tensor metadata but omits its required
  storage member, rather than silently returning invented weights;
- rejection of non-ZIP input by eager/lazy NPZ and PTH metadata readers.

The nonempty tensor's `data.pkl` bytes were generated with PyTorch's
`torch.save(..., pickle_protocol=2)`; its corresponding little-endian storage
and ZIP structure are reconstructed in the Rust test without a Python or model
download dependency. These small deterministic fixtures verify a concrete
storage-loading path, **not** representative model size, the full range of
PyTorch checkpoint formats, Kokoro speech output, or hostile ZIP metadata
handling. The ZIP 9 workflow runs its patched dependency compile and synthetic
reader tests in a **Linux, Windows and macOS matrix**; these are compatibility
checks, not a real asset load-and-synthesize test. Real Kokoro synthesis and
representative model/voice loading are still separate release gates.

The probe uses Python 3.12 on each runner and quotes Cargo source overrides
as TOML-compatible absolute paths with forward slashes, avoiding invalid
Windows backslash escapes. Its Cargo writes remain confined to the temporary
staged crate, and the matrix deliberately keeps platform failures separate
(`fail-fast: false`) so all three operating systems provide evidence.

The Linux matrix job also runs `cargo audit --file` against the **temporary,
fully resolved ZIP 9 migration lockfile** with **no `--ignore` flags**.
The probe fails if the temporary lockfile contains anything other than exactly
`zip 9.0.4` or if cargo-audit reports any known advisory. The Windows and
macOS matrix jobs retain the patched-source compilation and reader regression
checks, while Linux additionally validates the full temporary RustSec advisory
graph. The unignored audit is a **migration readiness check**, not an audit
of the unchanged production lockfile; production still contains ZIP 7.2.0
until the reviewed source replacement is actually committed.

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
