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

## Remaining upstream dependencies — NOT FIXED

`cargo audit` still reports three real advisories against each of two legacy
ZIP versions (six findings). **The presence of these advisories is not a false
positive and a green audited CI job does not mean the dependencies are patched.**

| ZIP dependency | Transitive consumers | Why not simply update |
| --- | --- | --- |
| zip 7.2.0 | any-tts 0.1.3, candle-core 0.10.2 | Upstream versions depend on ZIP 7. |
| zip 8.6.0 | docx-rs 0.4.22 | Upstream published crate depends on ZIP 8. |

Affected advisories:
- [RUSTSEC-2026-0367](https://rustsec.org/advisories/RUSTSEC-2026-0367) — permission-bit preservation in ZIP extraction
- [RUSTSEC-2026-0368](https://rustsec.org/advisories/RUSTSEC-2026-0368) — symlink escape in ZIP extraction
- [RUSTSEC-2026-0369](https://rustsec.org/advisories/RUSTSEC-2026-0369) — symlink traversal during ZIP extraction

These affect ZIP archive extraction APIs. The app's **direct** runtime archive
extractor now uses patched ZIP 9.0.4. Document export primarily *writes* DOCX
archives, but this does not prove arbitrary transitive extraction paths are
unreachable in all runtime configurations. In particular, upstream speech/model
packages need continued review. Treat input archives as untrusted.

## Temporary audit policy

The Security workflow ignores **only** the three ZIP advisory IDs above.
Before invoking cargo-audit, `scripts/check-transitive-zip-audit.mjs`:

1. Requires the patched direct ZIP, rustls and quinn-proto versions.
2. Requires the updater's legacy ZIP dependency to be absent.
3. Fails on **any new legacy ZIP version or transitive consumer**.
4. **Expires on 2026-11-17** so this exception cannot silently persist.

All other RustSec advisories, and all NPM high-severity findings, continue to
fail the dependency audit. The existing unmaintained/warning advisories remain
visible; they are not silently added to the exception.

## Required follow-up before claiming fully resolved

- Update any-tts/candle-core and docx-rs to versions depending on patched ZIP,
  or replace/vendoring them after compatibility and security verification.
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
