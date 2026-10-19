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

## Candidate upstream source provenance

The ZIP 9 probe now compares all files in each Cargo-verified registry source
tree against the disposable patched copy. It rejects **any unexpected modified,
added or deleted file**, or any source symlink. The allowed changed paths are
exactly `Cargo.toml` and `src/models/kokoro/model.rs` for `any-tts`; and
`Cargo.toml`, `src/npy.rs`, `src/pickle.rs` for `candle-core`.

Each Linux, Windows and macOS CI job uploads a short-lived evidence artifact
containing a deterministic `zip9-source-provenance.json` and one unified diff
per patched crate. The report records Cargo.lock's registry package checksum,
original and patched source-tree SHA-256, source file count, and original and
patched SHA-256 for each changed file. It does **not** upload unreviewed full
crate source snapshots, or claim these temporary files are production-vendored
and audited. The evidence can be inspected when creating reviewed, pinned,
license-compliant production replacements. CI artifacts are retained 14 days.

A red result is **evidence of a migration blocker**, not permission to
disable the actual security gate. The probe does not test full speech output,
ZIP member semantics, Windows/macOS runtime behavior, or package provenance.

## Source-pinned production replacement candidate

The Linux compatibility workflow also runs:

```sh
python scripts/probe-zip9-upgrade.py --audit --candidate-dir /tmp/zip9-production-candidate
```

The candidate directory contains a staged `src-tauri` source tree and
`Cargo.lock`, a `Cargo.toml` with **portable** `[patch.crates-io]` path
overrides to the two exact patched source directories under `vendor/`,
the original upstream license files, and `review/` provenance JSON plus
minimal unified diffs. Every vendored source tree must match its reviewed
patched-tree SHA-256 before the candidate can be built.

**Artifact integrity check (required).** Earlier uploaded candidates omitted
hidden registry metadata by default. An inspection of the Linux artifact from
PR #312 found 86 / 89 `any-tts` files and 104 / 106 `candle-core`
files; neither reconstructed source tree matched its recorded SHA-256.
These artifacts **must not be promoted** even though their pre-upload
build and audit jobs succeeded. The workflow now explicitly enables hidden
files for candidate uploads, downloads each uploaded artifact again, and
independently checks its full source-file count, patched source-tree and
individual patched-file SHA-256, allowed patch list, exact path-pinned
Cargo.toml and ZIP 9-only candidate lockfile. The verifier's negative test
removes hidden metadata and requires rejection. Any mismatch fails the CI
job; passing before-upload tests alone is insufficient.

The workflow now generates the same **source-pinned candidate separately on
Linux, Windows and macOS**. Each runner re-resolves with the candidate's own
manifest patches (without CLI overrides), requires only ZIP 9.0.4, runs
`cargo check --locked --lib` and `cargo test --locked --lib` using that
candidate, and uploads an OS-specific source-pinned artifact for seven days.
Linux additionally performs a second **unignored RustSec audit** of its
candidate lockfile. These checks exercise the patched production dependency
graph itself, not only a standalone tensor-reader test. A failed hash, Cargo
build, test or audit blocks the workflow, and no validated candidate artifact
is published for that runner.

The artifact is **not deployed or merged into production**. Its paths and
upstream license terms require review before promoting `vendor/` and the
corresponding manifest/lockfile. The final migration must run real Kokoro
model and voice inference tests, plus full Windows/macOS/Linux Rust, UI,
installer, update and security CI using the *committed* dependencies,
then demonstrate an **unignored production** cargo audit. Until then ZIP
7.2.0 remains in the production graph and issue #258 remains open.

## Production security gate during the final dependency switch

Production Security CI now has an explicit, **fail-closed** ZIP dependency
mode selector (`scripts/check-zip-audit-mode.mjs`). In the current **legacy**
mode, Cargo.lock must contain only ZIP 7.2.0 and 9.0.4, the old
`any-tts 0.1.3` and `candle-core 0.10.2` must still be exact registry
packages, and the existing restricted, expiring RustSec exceptions and
registry-source extraction inspection continue unchanged.

Once production `Cargo.toml` actually contains exact path-pinned reviewed
`vendor/any-tts-0.1.3` and `vendor/candle-core-0.10.2` sources, and the
**committed** production Cargo.lock contains **only ZIP 9.0.4**, Security CI
automatically selects **remediated** mode. It verifies exact local package
identities and source paths through `cargo metadata --locked`, inspects the
vendored Rust sources for forbidden archive extraction calls, and runs:

```sh
cargo audit --file src-tauri/Cargo.lock
```

**No advisory ignores** are passed in remediated mode. Partial transitions,
unreviewed ZIP versions, missing local source patches, or a mixed graph fail
closed rather than silently selecting a permissive audit. This conditional
gate does **not** itself promote the vendor artifacts, change the production
lockfile, prove model authenticity, or resolve the three existing advisories
until the source replacement is actually committed. The temporary legacy
exceptions must be deleted when the migration and final tests are complete.

## Isolated source-replacement review branch

A dedicated, review-only GitHub Actions workflow stages the verified ZIP 9
source-pinned candidate on the branch `security/zip9-source-replacement`.
The build job has **read-only** repository permissions and independently
generates, compiles, tests, RustSec-audits (no ignores), and SHA-256-verifies
the complete candidate before uploading it with hidden source files included.

Only a second job with branch-scoped write permission downloads the verified
artifact and runs `scripts/promote-zip9-production.py --apply`. The staging
script independently reruns artifact checks, refuses any unreviewed
`src-tauri/Cargo.toml` modifications or dirty vendor directories,
retains the exact checked-in-style ZIP 9 Cargo.lock, and includes third-party
source attribution and the bundled Apache-2.0 license text. The second job
may commit **only vendor sources and src-tauri Cargo inputs** to the review
branch. It must never push to main or merge a pull request.

Source files for any-tts 0.1.3 declare `MIT OR Apache-2.0`, but its
published registry crate does not bundle a top-level LICENSE; document and
review the Apache-2.0 option and upstream attribution before approving the
source replacement. Model weights are not vendored. The migration remains
**NOT REMEDIATED** until the branch's complete CI (including unignored
production RustSec audit), Kokoro real-model WAV test, and licensing review
pass and its changes are deliberately approved and merged.

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
