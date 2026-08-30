# Android Native Build Blockers

This file tracks native Android build blockers found by the `Android Mobile Validation` workflow and the fixes applied during the Phase 1 port.

## Cleared: Tauri Android project generation

`tauri android init` completes successfully in GitHub Actions with Node 24, Java 17, Android SDK/NDK, and Rust Android targets.

## Cleared: OpenSSL cross-compilation

Initial ARM64 builds stopped in `openssl-sys` because desktop/native TLS pulled a system OpenSSL dependency into the Android cross-compile.

Fix:

- disable Reqwest default features;
- use Rustls native roots;
- do not require a manually bundled Android OpenSSL sysroot.

## Cleared by target split: Candle FP16 TTS dependency

After OpenSSL was removed, the Android compiler reached `any-tts -> Candle -> gemm-f16` and failed on ARM half-precision assembly requiring `fullfp16`.

The Android app shell does not require local Kokoro TTS during Phase 1, and globally enabling an optional ARM ISA would unnecessarily narrow supported devices.

Fix:

- keep `any-tts` desktop-only;
- keep the desktop Kokoro implementation unchanged;
- return an explicit mobile unsupported response until the mobile inference/voice phase.

## Preventive split: Whisper

`whisper-rs` is also a native speech runtime and is not required to prove the Phase 1 Android app shell.

Fix:

- keep `whisper-rs` desktop-only;
- compile a mobile transcription stub with the same API contract;
- enable a validated Android speech runtime later in the mobile inference phase.

## Policy

Native AI features are added to mobile only after their backend is validated on Android ARM64 and does not silently require desktop system libraries or optional CPU features. The Android shell should compile and run independently of Phase 2 model/voice features.
