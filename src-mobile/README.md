# OpenMindAI Mobile

Flutter application for Android and iOS. Mobile code lives under `src-mobile/` and stays independent from the React/Tauri desktop application.

## Functional scope

OpenMindAI Mobile is local-first. AI inference, normal chat history, voice transcription, document processing, vision input and Canvas generation run on the device. Search and Research retrieve public web evidence when selected. Firebase Analytics is initialized for app-level usage analytics only; OpenMindAI does not send prompts, responses, attachments, local model files or chat history through its Firebase service.

Implemented end-to-end functionality:

- first-run onboarding, capability permissions and full license acceptance;
- storage-saving first-run model provisioning;
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
- Firebase Core + Analytics for Android and iOS;
- OpenMindAI Canvas on-device SVG image synthesis using the selected installed OpenMindAI model, with safe-SVG validation, preview, Save As and system Share.

Canvas deliberately uses local vector synthesis instead of silently calling a paid/cloud image API. A heavier diffusion runtime can be added later as an optional performance/model package without changing local inference behavior.

## First run and model footprint

OpenMindAI no longer auto-downloads a multi-gigabyte model just because a phone has more RAM. A new install starts with **OpenMindAI Nano**, approximately **429 MB**. Larger models are available only when the user explicitly installs them from Model Manager.

1. Welcome and capability permissions.
2. Local-AI usage instructions.
3. Full OpenMindAI Apache 2.0 license read/accept.
4. Device RAM/storage inspection.
5. OpenMindAI Nano is downloaded to app-private storage and verified.
6. Chat opens.

After setup completes, `mobile_onboarding_complete_v1` is stored locally. Later launches open Chat directly.

### Optional local models

- OpenMindAI Nano — about 0.43 GB — first-run/default lightweight model
- OpenMindAI Swift — about 1.28 GB — optional
- OpenMindAI Core — about 2.50 GB — optional
- OpenMindAI Titan — about 5.20 GB — optional
- OpenMindAI Reasoning Mini — about 1.12 GB — optional
- OpenMindAI Reasoning — about 4.68 GB — optional
- OpenMindAI Lens — about 2.78 GB plus its multimodal projector — optional and installed only for vision use

The current catalog already uses mobile-oriented Q4 quantization for the main models. Upstream repository names and raw filenames are internal provisioning metadata and are not rendered in normal user-facing screens.

Models are not bundled inside the APK. They are downloaded on demand into app-private storage and can be removed independently, so installing OpenMindAI does not automatically consume the combined size of the model catalog.

## Firebase configuration

The native app identity is:

```text
com.openmindai.mobile
```

Firebase configuration is kept in:

- `android/app/google-services.json`
- `ios/Runner/GoogleService-Info.plist`
- `lib/firebase_options.dart`

`FirebaseService` initializes Firebase Core and Analytics without making local AI startup depend on Firebase connectivity. Firebase initialization failure does not prevent offline/local inference from opening.

`tool/prepare_platforms.dart` validates that the committed Android package, iOS bundle, Firebase project IDs and Firebase application IDs still match before a local build proceeds. This prevents a regenerated native host from silently using the wrong Firebase app.

## Local development and APK builds

Flutter mobile builds are intentionally local. The repository does not run a dedicated GitHub Actions Flutter/APK workflow.

The dependency lockfile currently requires **Dart 3.12+** and **Flutter 3.44+**. `pubspec.yaml` declares the same minimum toolchain so dependency resolution and project metadata stay consistent.

The Android and iOS hosts are committed. From `src-mobile/` normally run:

```bash
flutter pub get
dart run tool/prepare_platforms.dart
dart format lib test tool
flutter analyze
flutter test
flutter build apk --debug
```

If native hosts are deliberately regenerated, restore/retain the committed Firebase configuration files and run `dart run tool/prepare_platforms.dart` before building. The preparation command now fails with a clear error if the Firebase files or application identities do not match.

For a smaller device-specific Android artifact, Flutter can build split APKs instead of one universal APK:

```bash
flutter build apk --release --split-per-abi
```

For a normal release APK after local signing/configuration is ready:

```bash
flutter build apk --release
```

For iOS development on macOS:

```bash
flutter build ios --simulator --debug
```

`tool/prepare_platforms.dart` preserves the `com.openmindai.mobile` identity and configures Android permissions, Firebase Gradle integration, API/NDK compatibility, Java 17/desugaring, TTS discovery, and iOS permission descriptions/deployment target.

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

Phones have tighter memory, battery and thermal limits than desktops. OpenMindAI therefore keeps the first-run model conservative while allowing users to install or remove larger models explicitly.
