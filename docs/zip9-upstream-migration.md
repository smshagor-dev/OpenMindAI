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

## Ongoing real-voice regression coverage

The `Kokoro Real Voice Inference` workflow runs automatically for pull
requests that change the pinned TTS Rust/runtime sources, model/package
loading, Cargo dependencies, or the real inference test itself. It also
runs on applicable `main` pushes, every Monday at 05:31 UTC, and by
manual dispatch. This intentionally avoids large pretrained downloads
on unrelated UI or documentation changes. Concurrent runs for the same
pull request are canceled when superseded.

The genuine checkpoint, voice and config are all SHA-256-pinned at an
immutable model revision. The config checksum
`5abb01e2403b072bf03d04fde160443e209d7a0dad49a423be15196b9b43c17f`
was verified on the successful Linux inference run #39274446841 and is now
required before Rust/Candle loads the package. Evidence includes the
pinned config checksum alongside voice/model hashes and WAV duration,
format and signal metrics. A regression or inability to fetch the upstream
assets fails the job; no synthetic fallback can mark real inference green.

This is a Linux CPU end-to-end regression. Real voices, model availability
and the same inference route on Windows/macOS or multiple accelerators remain
separate validation subjects. The scheduled check and PR trigger are
additional protections rather than a replacement for full release testing.

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
