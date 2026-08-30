# Android ARM64 Build Gate

A successful Phase 1 native gate requires `tauri android init` followed by an ARM64 debug APK build on the CI Android toolchain. The gate intentionally excludes unvalidated mobile speech runtimes while preserving desktop speech behavior.
