# ZIP 9 production migration and verification

**Status: production migrated; legacy ZIP 7 exceptions retired.**

The ZIP 9 remediation began as an isolated upstream compatibility
experiment. The tracked desktop dependencies now use source-pinned,
reviewed copies of `any-tts 0.1.3` and `candle-core 0.10.2` which
have been minimally adapted to `zip 9.0.4`.

## Final production source requirements

- `src-tauri/Cargo.toml` pins the two local `vendor/` paths via
  `[patch.crates-io]`.
- `src-tauri/Cargo.lock` contains **exactly one ZIP crate, 9.0.4**.
  ZIP 7 and ZIP 8 are not part of the production graph.
- The vendored source snapshot is checked with
  `python3 scripts/verify-zip9-vendor.py` (approved SHA-256 tree hashes,
  patched source files, license and attribution notices).
- `node scripts/check-zip-audit-mode.mjs` and
  `node scripts/check-zip-audit-mode.mjs --metadata ...` verify the
  exact lockfile, patched package identities, local source paths and
  restricted ZIP archive handling.
- Security CI unconditionally runs
  `cargo audit --file src-tauri/Cargo.lock` without advisory ignores.
  Legacy ZIP 7 or a partial migration is a **failure**, not a fallback.

The original any-tts archive has one macOS Finder metadata file which
is deliberately omitted from the committed source; all its code and
upstream Markdown notices are kept. Reproducible full source evidence
is retained by the candidate workflows.

## Real Kokoro voice verification

The Linux CPU workflow **Kokoro Real Voice Inference** downloads
`hexgrad/Kokoro-82M` at fixed revision
`04393022a04dcb701fd2061b8d5dbe23ac7be751`, validates the
checkpoint and voice against SHA-256, and runs the production app's
Rust/Candle `media_smoke speak` example. The generated WAV is then
checked for its format, duration and non-silent RMS.

[Real inference run](https://github.com/smshagor-dev/OpenMindAI/actions/runs/39274446841)
passed with 24,000 Hz, mono, 16-bit PCM, **3.65 seconds** and
**RMS 0.0485709**. The workflow uploads only safe JSON evidence; it
does not commit third-party weights or the generated audio.

Real voice output on Linux is proven; different CPU/GPU platforms and
additional voices still require their own smoke coverage.

## Historical work and source evidence

The review history traces the original package adaptation, registry
source SHA-256 checks, ZIP NPZ/PTH regression fixtures, candidate Cargo
lockfile generation, and downloadable artifact integrity:

- PRs #305–#308: checked ZIP 9 compatibility, unignored candidate audit
  and source-pinned migration candidate.
- PRs #312–#313: validated candidates on desktop operating systems and
  prevented missing hidden files in uploaded source artifacts.
- PR #314: installed the fail-closed production security transition gate.
- PR #317: committed the actual reviewed source replacement with
  multi-platform CI and production no-ignore RustSec audit.
- PR #358: validated authentic Kokoro speech synthesis and non-silent WAV.
  
The historical migration scripts continue to test the upstream adapter
against the original registry dependency graph. They do **not** authorize
legacy ZIP dependencies or any production RustSec advisory ignores.
