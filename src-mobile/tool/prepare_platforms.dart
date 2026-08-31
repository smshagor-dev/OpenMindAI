import 'dart:io';

Future<void> main() async {
  await _patchAndroidManifest();
  await _patchAndroidMinSdk();
  await _patchIosInfoPlist();
  await _patchIosDeploymentTarget();
  stdout.writeln('OpenMindAI mobile platform configuration is ready.');
}

Future<void> _patchAndroidManifest() async {
  final file = File('android/app/src/main/AndroidManifest.xml');
  if (!await file.exists()) {
    throw StateError(
      'Android host is missing. Run flutter create --platforms=android,ios . first.',
    );
  }
  var value = await file.readAsString();
  const permissions = '''
    <uses-permission android:name="android.permission.INTERNET" />
    <uses-permission android:name="android.permission.CAMERA" />
    <uses-permission android:name="android.permission.RECORD_AUDIO" />
    <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
''';
  if (!value.contains('android.permission.CAMERA')) {
    value = value.replaceFirst('>\n', '>\n$permissions');
  }

  const ttsQueries = '''
    <queries>
        <intent>
            <action android:name="android.intent.action.TTS_SERVICE" />
        </intent>
    </queries>
''';
  if (!value.contains('android.intent.action.TTS_SERVICE')) {
    value = value.replaceFirst('\n    <application', '$ttsQueries\n    <application');
  }
  await file.writeAsString(value);
}

Future<void> _patchAndroidMinSdk() async {
  final file = File('android/app/build.gradle.kts');
  if (!await file.exists()) return;
  var value = await file.readAsString();
  value = value.replaceFirst(
    RegExp(r'minSdk\s*=\s*flutter\.minSdkVersion'),
    'minSdk = 21',
  );
  await file.writeAsString(value);
}

Future<void> _patchIosInfoPlist() async {
  final file = File('ios/Runner/Info.plist');
  if (!await file.exists()) {
    throw StateError(
      'iOS host is missing. Run flutter create --platforms=android,ios . first.',
    );
  }
  var value = await file.readAsString();
  const capabilities = '''
\t<key>NSCameraUsageDescription</key>
\t<string>OpenMindAI uses the camera only when you choose to attach an image or document.</string>
\t<key>NSMicrophoneUsageDescription</key>
\t<string>OpenMindAI uses the microphone only when you choose voice input.</string>
\t<key>NSPhotoLibraryUsageDescription</key>
\t<string>OpenMindAI accesses photos only when you choose images to attach to a conversation.</string>
''';
  if (!value.contains('NSCameraUsageDescription')) {
    value = value.replaceFirst('\n</dict>', '$capabilities\n</dict>');
  }
  await file.writeAsString(value);
}

Future<void> _patchIosDeploymentTarget() async {
  const target = '15.6';

  final podfile = File('ios/Podfile');
  if (await podfile.exists()) {
    var value = await podfile.readAsString();
    if (value.contains("platform :ios, '")) {
      value = value.replaceFirst(
        RegExp(r"platform :ios, '[^']+'"),
        "platform :ios, '$target'",
      );
    } else {
      value = "platform :ios, '$target'\n$value";
    }
    await podfile.writeAsString(value);
  }

  final project = File('ios/Runner.xcodeproj/project.pbxproj');
  if (await project.exists()) {
    var value = await project.readAsString();
    value = value.replaceAll(
      RegExp(r'IPHONEOS_DEPLOYMENT_TARGET = [0-9.]+;'),
      'IPHONEOS_DEPLOYMENT_TARGET = $target;',
    );
    await project.writeAsString(value);
  }

  final frameworkInfo = File('ios/Flutter/AppFrameworkInfo.plist');
  if (await frameworkInfo.exists()) {
    var value = await frameworkInfo.readAsString();
    value = value.replaceFirst(
      RegExp(
        r'(<key>MinimumOSVersion</key>\s*<string>)[^<]+(</string>)',
        multiLine: true,
      ),
      '\$1$target\$2',
    );
    await frameworkInfo.writeAsString(value);
  }
}
