# OpenMindAI Mobile

Flutter mobile application for Android and iOS. The mobile code is isolated under `src-mobile/` so the existing React/Tauri desktop application and desktop CI remain independent.

## First-run flow

1. Welcome and required capability permissions.
2. Local-AI usage instructions.
3. Full OpenMindAI Apache 2.0 license read/accept step.
4. Device inspection and RAM/storage-aware model recommendation.
5. Chat opens with the recommended OpenMindAI model selected.

After onboarding completes, `mobile_onboarding_complete_v1` is stored locally. Later launches bypass onboarding and open Chat directly.

## Model names

The mobile UI intentionally uses the same public model names as desktop, for example:

- OpenMindAI Nano
- OpenMindAI Swift
- OpenMindAI Core
- OpenMindAI Titan
- OpenMindAI Reasoning Mini
- OpenMindAI Reasoning
- OpenMindAI Lens

Upstream repository/model names are not part of the mobile presentation layer. Runtime code receives only the stable OpenMindAI model id.

## Current foundation

Implemented in this first mobile foundation:

- first-run routing and persistent onboarding state;
- camera, microphone, notification and iOS photo permission requests;
- no broad Android storage permission; files use system pickers;
- instructions and full license agreement UI;
- Android/iOS device profile detection with physical RAM and free disk space;
- device-based recommended model selection;
- ChatGPT-inspired OpenMindAI chat layout with drawer/history, model picker, modes, attachments and composer;
- locally persisted conversation history;
- camera, photo and file attachment picking;
- native inference `MethodChannel` contract at `openmindai.mobile/inference`.

The Android/iOS native local-inference implementation is the next runtime layer. The Flutter UI does not silently fall back to a paid cloud model when the native bridge is unavailable.

## Bootstrap Android/iOS hosts

This branch starts with the Flutter-owned application source and keeps generated platform hosts out until the local inference bridge is selected. From `src-mobile/`, generate standard Flutter platform hosts with a current Flutter SDK:

```bash
flutter create --platforms=android,ios --org com.openmindai --project-name openmindai_mobile .
flutter pub get
```

Then apply the platform permission/native bridge requirements below before building.

### Android permissions

The generated `android/app/src/main/AndroidManifest.xml` should declare only capabilities used by features:

```xml
<uses-permission android:name="android.permission.CAMERA" />
<uses-permission android:name="android.permission.RECORD_AUDIO" />
<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
```

File access uses Android's system picker/SAF, so `MANAGE_EXTERNAL_STORAGE` is intentionally not requested.

### iOS permission descriptions

Add these keys to `ios/Runner/Info.plist`:

```xml
<key>NSCameraUsageDescription</key>
<string>OpenMindAI uses the camera when you choose to attach a photo or document.</string>
<key>NSMicrophoneUsageDescription</key>
<string>OpenMindAI uses the microphone when you choose voice input.</string>
<key>NSPhotoLibraryUsageDescription</key>
<string>OpenMindAI accesses selected photos only when you attach them to a conversation.</string>
```

## Native inference bridge

Flutter calls:

- channel: `openmindai.mobile/inference`
- method: `generate`
- arguments: `modelId`, `mode`, `attachments`, `messages`
- result: final assistant text

The planned native bridge should keep model files and inference local, expose cancellation/streaming in the next iteration, and use the same catalog IDs as desktop.
