# Rust dependency audit: temporary ZIP advisory exceptions (2026-10-17)

## Remediated and enforced

- **quinn-proto**: 0.11.17 -> 0.11.19 (addresses RUSTSEC-2026-0338/0340/0343).
- **rustls**: 0.23.45 -> 0.23.46 (addresses RUSTSEC-2026-0346/0347/0348/0349).
- **Direct zip**: 2.4.2 -> 9.0.4. This protects OpenMindAI's own
  runtime installer, which calls `ZipArchive::extract`.
- **tauri-plugin-updater**: pinned to 2.10.1 (to match the installed JS plugin);
  legacy ZIP update support is disabled with `default-features = false` and
  `features = ["rustls-tls"]`, eliminating the old zip 4.x lockfile entry.
- The manually extracted diffusion runtime strips Unix SUID, SGID and sticky
  permission bits instead of carrying these bits into new files.
- **DOCX export**: uses a minimal WordprocessingML package writer and patched
  direct ZIP 9.0.4, removing `docx-rs 0.4.22`, `zip 8.6.0`, and their unused
  lockfile dependencies. Existing headings, paragraphs, lists, code and tables
  are supported, with explicit XML escaping and ZIP package regression tests.

## Runtime installer ZIP preflight (defense in depth)

The official llama.cpp ZIP installer now rejects unsafe archive members before
extracting with the patched ZIP 9.0.4 library. It rejects path escapes, duplicate
or empty paths, symbolic links and special-file types, SUID/SGID/sticky permission
bits, and archives advertising more than 20,000 entries or 4 GiB uncompressed.
Rejected archives are discarded from staging without replacing a working
installed runtime. Regression tests cover permission modes and resource bounds;
existing installer tests cover failed-extraction cleanup and preserving a
working runtime.

This reduces exposure in the **direct runtime installer**. It is **not** a
replacement for upstream ZIP 7 remediation and does not establish that every
archive-handling code path in a transitive crate is safe. The exception and its
original expiry remain in force.

## Remaining upstream dependencies — NOT FIXED

`cargo audit` still reports three real advisories against the remaining
ZIP 7.2.0 package. **These advisories are real: a green audited CI job with
exceptions does not mean that the dependency is patched.**

| ZIP dependency | Transitive consumers | Why not simply update |
| --- | --- | --- |
| zip 7.2.0 | any-tts 0.1.3, candle-core 0.10.2 | Upstream versions depend on ZIP 7. |

Affected advisories:
- [RUSTSEC-2026-0367](https://rustsec.org/advisories/RUSTSEC-2026-0367) — permission-bit preservation in ZIP extraction
- [RUSTSEC-2026-0368](https://rustsec.org/advisories/RUSTSEC-2026-0368) — symlink escape in ZIP extraction
- [RUSTSEC-2026-0369](https://rustsec.org/advisories/RUSTSEC-2026-0369) — symlink traversal during ZIP extraction

These affect ZIP archive extraction APIs. The app's **direct** runtime archive
extractor now uses patched ZIP 9.0.4. DOCX export now writes archives directly through ZIP 9.0.4 and no longer
links docx-rs or ZIP 8.6.0. However, upstream speech/model packages still
require investigation of all archive-handling paths. Treat input archives as
untrusted.

## Kokoro model-load boundary checks

Before local Kokoro TTS loads its voice and config files through upstream
`any-tts` / Candle, OpenMindAI now resolves the required `config.json`
and `af_heart.pt` paths and rejects symlinks escaping the expected model
package or voices directory, even when the targets remain elsewhere within
the portable root. ZIP-format `.pt` voice files also receive read-only
metadata screening through the patched direct ZIP 9 reader before upstream
loading: path enclosure, duplicate members, symlink/special member types,
privileged Unix mode bits, 4,096 entry limit, and 1 GiB declared uncompressed
size limit. Existing non-ZIP PyTorch assets remain supported.

This reduces the input surface of the **Kokoro voice loader only**; it does
not patch `zip 7.2.0`, prove that all upstream code paths avoid extraction,
or provide a sandbox. The remaining three RustSec advisories must not be
marked resolved based on these controls.

## Temporary audit policy

The Security workflow ignores **only** the three ZIP advisory IDs above.
Before invoking cargo-audit, `scripts/check-transitive-zip-audit.mjs`:

1. Requires the patched direct ZIP, rustls and quinn-proto versions.
2. Requires the updater's legacy ZIP dependency to be absent.
3. Fails on **any new legacy ZIP version or transitive consumer**, including
   reintroducing `docx-rs`.
4. **Expires on 2026-11-17** so this exception cannot silently persist.

All other RustSec advisories, and all NPM high-severity findings, continue to
fail the dependency audit. The existing unmaintained/warning advisories remain
visible; they are not silently added to the exception.

## Required follow-up before claiming fully resolved

- Update or safely replace `any-tts` / `candle-core` to avoid ZIP 7, or use
  a reviewed, maintained fork with a compatible patched archive dependency.
- Review their ZIP archive handling, including malicious filenames, symlinks,
  and inherited Unix permission bits. Do not depend on the temporary exception
  to establish safety.
- Remove the three ignores, remove the guard script, and run
  `cargo audit --file src-tauri/Cargo.lock` **without exceptions**.
- Run Linux/macOS/Windows build tests, archive extraction regression tests and
  a release updater smoke test.

**Release decision:** This exception is an explicitly time-limited risk
acceptance, not full dependency remediation. Re-evaluate before shipping a
security-sensitive production release.
