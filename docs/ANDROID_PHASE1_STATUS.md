# Android Phase 1 Status

The initial mobile foundation has been merged. The active continuation focuses on making the native Android ARM64 application compile without inheriting desktop-only AI/runtime dependencies.

## Verified

- Tauri Android project generation succeeds in CI.
- Shared React/TypeScript production build succeeds.
- Android-specific Tauri configuration is applied.
- Mobile Tauri permissions are separated from desktop permissions.
- Full PC/local workspace/terminal behavior is blocked on mobile in both frontend and Rust.
- Reqwest no longer depends on Android OpenSSL cross-compilation.

## Current native dependency policy

Phase 1 proves the application shell and mobile-safe product core first. Desktop-only AI runtimes are target-gated until their Android implementations are benchmarked and validated.

- Kokoro/`any-tts`: desktop-only during Phase 1.
- Whisper/`whisper-rs`: desktop-only during Phase 1.
- Android speech commands return explicit unsupported responses instead of pretending the desktop runtime is available.

## Next gates

1. Pass Android ARM64 debug APK compilation.
2. Resolve the next native dependency or platform compile error, if any.
3. Introduce Android-safe application storage instead of desktop portable-root assumptions.
4. Launch the debug APK on emulator/physical ARM64 hardware.
5. Validate setup, chat persistence, Projects navigation, Library, and Settings.
