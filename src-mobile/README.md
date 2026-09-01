# OpenMindAI Mobile

Flutter application for Android and iOS. Mobile code lives under `src-mobile/` and stays independent from the React/Tauri desktop application.

## Functional scope

OpenMindAI Mobile is local-first. The AI model, normal chat history, voice transcription, document processing, vision input and Canvas generation run on the device. Search and Research use the internet only to retrieve public evidence; the final model inference remains local.

Implemented end-to-end functionality:

- first-run onboarding, capability permissions and full license acceptance;
- device RAM/free-storage inspection and recommended model selection;
- app-private model download with resume, cancellation and SHA-256 verification when upstream LFS metadata provides a digest;
- local GGUF inference through `lib_llama_cpp`;
- resident text-model sessions with streamed output;
- Chat, Think, Search and Research modes;
- Stop and Regenerate;
- SQLite conversation history with automatic migration from the earlier SharedPreferences format;
- chat search, rename, delete, export and full-history cleanup;
- camera and photo input through OpenMindAI Lens + multimodal projector;
- PDF, DOCX, text, Markdown, source-code, JSON, YAML, CSV and other text attachment context;
- durable app-private copies of chat attachments so old chats do not depend on temporary picker paths;
- orphan attachment cleanup and storage usage controls;
- OpenMindAI Hear local Whisper voice dictation;
- OpenMindAI Speak device TTS read-aloud;
- optional completion notifications for long responses finishing while the app is inactive;
- real haptic-feedback setting used by important controls;
- System, Light and Dark themes plus compact chat spacing;
- local model install/delete/cancel manager;
- OpenMindAI Canvas on-device SVG image synthesis using the selected installed OpenMindAI model, with safe-SVG validation, preview, Save As and system Share.

Canvas deliberately uses local vector synthesis instead of silently calling a paid/cloud image API. A heavier diffusion runtime can be added later as an optional performance/model package without changing the local-only contract.

## First run

1. Welcome and capability permissions.
2. Local-AI usage instructions.
3. Full OpenMindAI Apache 2.0 license read/accept.
4. Device RAM/storage inspection and recommended local model.
5. The recommended model is downloaded to app-private storage and verified.
6. Chat opens.

After setup completes, `mobile_onboarding_complete_v1` is stored locally. Later launches open Chat directly.

## Local models

The mobile UI uses OpenMindAI product names. Upstream repository names and raw filenames are internal provisioning metadata and are not rendered in normal user-facing screens.

Current mobile model set:

- OpenMindAI Nano
- OpenMindAI Swift
- OpenMindAI Core
- OpenMindAI Titan
- OpenMindAI Reasoning Mini
- OpenMindAI Reasoning
- OpenMindAI Lens

Vision installs include the matching multimodal projector.

## Local development and APK builds

Flutter mobile builds are intentionally local. The repository no longer runs a dedicated GitHub Actions Flutter/APK workflow.

From `src-mobile/`:

```bash
flutter create --platforms=android,ios --org com.openmindai --project-name openmindai_mobile --no-pub .
dart run tool/prepare_platforms.dart
flutter pub get
dart format lib test tool
flutter analyze
flutter test
flutter build apk --debug
```

For a release APK after local signing/configuration is ready:

```bash
flutter build apk --release
```

For iOS development on macOS:

```bash
flutter build ios --simulator --debug
```

`tool/prepare_platforms.dart` configures the generated hosts for the dependencies used by the app, including Android permissions, API/NDK compatibility, Java 17/desugaring, TTS discovery, and iOS permission descriptions/deployment target.

### Android permissions

- `INTERNET`
- `CAMERA`
- `RECORD_AUDIO`
- `POST_NOTIFICATIONS`

Broad storage access such as `MANAGE_EXTERNAL_STORAGE` is deliberately not requested. Files use system pickers and durable chat copies are stored inside the application sandbox.

### iOS permissions

- camera;
- microphone;
- photo library;
- notification permission is requested through the app capability flow.

## Runtime notes

The baseline `lib_llama_cpp` mobile prebuilts use the supported mobile CPU runtime by default. GPU-specific Android Vulkan and iOS Metal packaging remains an optional optimization rather than a correctness dependency.

Phones have tighter memory, battery and thermal limits than desktops. OpenMindAI therefore recommends a conservative model from detected device capacity while still allowing users to install or remove alternatives.
