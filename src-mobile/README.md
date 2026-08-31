# OpenMindAI Mobile

Flutter application for Android and iOS. Mobile code lives under `src-mobile/` so the React/Tauri desktop app remains independent.

## First run

1. Welcome and capability permissions.
2. Local-AI usage instructions.
3. Full OpenMindAI Apache 2.0 license read/accept.
4. Device RAM/storage inspection and recommended local model.
5. The recommended model is downloaded to app-private storage and verified.
6. Chat opens.

After setup completes, `mobile_onboarding_complete_v1` is stored locally. Later launches open Chat directly.

## Local models

The mobile UI uses the same OpenMindAI product names as desktop. Upstream repository names and raw filenames are internal provisioning metadata and must not be rendered in user-facing screens.

Current mobile local-model set:

- OpenMindAI Nano
- OpenMindAI Swift
- OpenMindAI Core
- OpenMindAI Titan
- OpenMindAI Reasoning Mini
- OpenMindAI Reasoning
- OpenMindAI Lens

Models are resolved from their configured upstream repositories, downloaded to app-private application support storage, written through a temporary `.part` file, and SHA-256 verified when upstream LFS metadata exposes a digest. Vision installs include the matching multimodal projector.

## Local inference

`lib_llama_cpp` is the direct Android/iOS llama.cpp runtime. Chat does not require a paid cloud AI API.

Implemented paths:

- local GGUF model mounting;
- streamed token output;
- Stop and Regenerate;
- Chat and Think modes;
- local conversation persistence;
- image input through OpenMindAI Lens + mmproj;
- camera and photo attachments;
- PDF structured-text extraction;
- text, Markdown, source-code, JSON/YAML/CSV and other text attachments;
- Search and Research modes that retrieve public web evidence and pass it to the local model;
- local model install/progress/cancel/delete manager.

Search and Research require internet access. The AI model itself still runs locally.

## Bootstrap Android/iOS hosts

Generated Flutter host files are reproducible and are intentionally not mixed with the desktop source. From `src-mobile/`:

```bash
flutter create --platforms=android,ios --org com.openmindai --project-name openmindai_mobile --no-pub .
dart run tool/prepare_platforms.dart
flutter pub get
flutter analyze
flutter test
flutter build apk --debug
```

`tool/prepare_platforms.dart` configures:

### Android

- `INTERNET`
- `CAMERA`
- `RECORD_AUDIO`
- `POST_NOTIFICATIONS`

Broad storage access such as `MANAGE_EXTERNAL_STORAGE` is deliberately not requested. Files use the system picker.

### iOS

- `NSCameraUsageDescription`
- `NSMicrophoneUsageDescription`
- `NSPhotoLibraryUsageDescription`
- iOS deployment target 13.0 in the generated Podfile

## CI

`.github/workflows/mobile-flutter.yml` validates the mobile source separately from desktop CI. It generates clean Android/iOS hosts, applies platform configuration, resolves dependencies, checks formatting, runs `flutter analyze` and tests, builds an Android debug APK, and performs a no-codesign iOS debug build.

## Runtime notes

The pub.dev `lib_llama_cpp` mobile prebuilts are CPU builds by default. GPU-specific Android Vulkan and iOS Metal assets are a separate optimization path and are not assumed by the baseline mobile build.

Large models remain optional because phones have tighter memory and thermal limits than desktop systems. OpenMindAI selects a conservative default from detected RAM, while the Models screen lets the user install or remove alternatives.
