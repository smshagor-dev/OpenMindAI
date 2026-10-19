# ZIP 7 RustSec exception policy — retired

**Status: retired after the production ZIP 9 source migration.**

The original scoped policy temporarily excluded RUSTSEC-2026-0367,
RUSTSEC-2026-0368 and RUSTSEC-2026-0369 for `zip 7.2.0`, with a planned
2026-11-17 expiration. That legacy risk acceptance is **no longer active**.

Production ZIP remediation is now protected by:

- `src-tauri/Cargo.toml`: exact path-pinned `any-tts 0.1.3` and
  `candle-core 0.10.2` sources under `vendor/`.
- `src-tauri/Cargo.lock`: only `zip 9.0.4`. All `zip 7.x` and
  `zip 8.x` transitive packages are absent.
- `scripts/check-zip-audit-mode.mjs`: fails closed on legacy ZIP,
  unexpected crate versions, missing patched local sources and changed
  `rustls`, `quinn-proto`, updater and DOCX invariants.
- `scripts/verify-zip9-vendor.py`: verifies the exact source-tree hashes,
  required upstream files and third-party attribution/license notices.
- `.github/workflows/security.yml`: always runs
  `cargo audit --file src-tauri/Cargo.lock`, **without `--ignore` flags**.

## Supporting validation

- [Production ZIP 9 source migration PR #317](https://github.com/smshagor-dev/OpenMindAI/pull/317)
  merged as `48246fb60c70d7cd9acb5bd5953290b80903e2fa`.
  Windows, macOS and Linux Rust checks, NSIS, native Vulkan/CXX,
  Security/CodeQL and no-exception cargo audit passed.
- [Real Kokoro voice PR #358](https://github.com/smshagor-dev/OpenMindAI/pull/358)
  merged as `6046730b5892fa2f3d39e145f0e376564a339e76`.
  The authenticated Kokoro checkpoint and voice generated a real,
  non-silent 24 kHz mono PCM16 WAV. Evidence recorded 3.65 s of audio
  and RMS 0.0485709; see the workflow run and downloadable JSON evidence.

The earlier legacy policy was intentionally time-limited and is kept in
Git history for investigation. This file now documents retirement rather
than permitting additional exceptions. A future RustSec finding must fail
the audit until fixed or separately reviewed under a **new, explicit**
policy; the old ZIP 7 ignore list must not be reintroduced.

Production inference success here is a specific Linux CPU test, not a
claim of complete real-model speech coverage across all hardware.
