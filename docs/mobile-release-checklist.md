# Mobile Release Completion Gates

OpenMindAI mobile is considered production-ready only when every gate below passes on the release commit.

## Android

- Shared frontend lint/build passes.
- Rust dependency lock passes.
- ARM64 debug APK builds and is uploaded.
- ARM64 release AAB builds and is uploaded.
- x86_64 emulator APK installs and remains alive after launch.
- Startup logcat contains no OpenMindAI fatal exception, fatal signal, panic, or native-linker failure.
- App-private storage is used for databases, downloads, artifacts, logs, and model files.
- Local GGUF text generation works through the embedded llama.cpp engine.
- Backgrounding releases cached model memory safely and foregrounding can lazily reload it.
- Conversations, regenerate, projects, model manager, settings, attachments, and supported integrations work from the mobile UI.
- A physical ARM64 Android device passes install, cold-start, model download/import, generation, background/foreground, and relaunch smoke tests.
- Store signing credentials are supplied only through protected CI/release secrets; no keystore or password is committed.

## iOS

- Shared frontend lint/build passes.
- Rust dependency lock passes.
- Tauri/Xcode simulator target initializes and packages an `.app` artifact.
- Native llama.cpp binding compiles for Apple mobile targets with Metal enabled where supported.
- Local GGUF text generation is wired through the same mobile chat contract used by Android.
- App-private storage is used for databases, downloads, artifacts, logs, and model files.
- Conversations, regenerate, projects, model manager, settings, attachments, and supported integrations work from the mobile UI.
- A physical iPhone/iPad passes install, cold-start, model download/import, generation, background/foreground, and relaunch smoke tests.
- App Store signing/provisioning credentials are supplied only through protected CI/release secrets.

## Release

- Main CI, security, Android validation, Android emulator smoke, Android release packaging, and iOS simulator/native validation are green on the same release commit.
- Generated Android/iOS artifacts are retained for release verification.
- Known mobile-only unsupported desktop capabilities are explicitly disabled instead of failing at runtime.
- Release notes document minimum OS/device requirements and the tested mobile model-size policy.
