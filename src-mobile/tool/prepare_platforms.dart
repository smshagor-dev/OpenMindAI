import 'dart:convert';
import 'dart:io';

const _appId = 'com.openmindai.mobile';
const _firebaseProjectId = 'open-mind-ai-b38e2';
const _androidFirebaseAppId =
    '1:369874806080:android:f0749aaa2cc210d95c86a5';
const _iosFirebaseAppId = '1:369874806080:ios:65c7b85e4f5ff5775c86a5';

Future<void> main() async {
  await _patchAndroidManifest();
  await _patchAndroidBuild();
  await _patchAndroidSettings();
  await _patchAndroidMainActivity();
  await _writeAndroidNotificationIcon();
  await _patchIosInfoPlist();
  await _patchIosDeploymentTarget();
  await _patchIosBundleIdentifiers();
  await _validateFirebaseConfiguration();
  stdout.writeln(
    'OpenMindAI mobile platform configuration and Firebase identity are ready for local builds.',
  );
}

Future<void> _patchAndroidManifest() async {
  final file = File('android/app/src/main/AndroidManifest.xml');
  if (!await file.exists()) {
    throw StateError(
      'Android host is missing. Restore the committed android/ host before building.',
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
    value = value.replaceFirst(
      '\n    <application',
      '$ttsQueries\n    <application',
    );
  }
  await file.writeAsString(value);
}

Future<void> _patchAndroidBuild() async {
  final file = File('android/app/build.gradle.kts');
  if (!await file.exists()) {
    throw StateError('Android Gradle app configuration is missing.');
  }
  var value = await file.readAsString();

  if (!value.contains('id("com.google.gms.google-services")')) {
    value = value.replaceFirst(
      'id("com.android.application")',
      'id("com.android.application")\n    id("com.google.gms.google-services")',
    );
  }
  value = value.replaceFirst(
    RegExp(r'namespace\s*=\s*"[^"]+"'),
    'namespace = "$_appId"',
  );
  value = value.replaceFirst(
    RegExp(r'applicationId\s*=\s*"[^"]+"'),
    'applicationId = "$_appId"',
  );
  value = value.replaceFirst(
    RegExp(r'compileSdk\s*=\s*(?:flutter\.compileSdkVersion|\d+)'),
    'compileSdk = 37',
  );
  value = value.replaceFirst(
    RegExp(r'ndkVersion\s*=\s*(?:flutter\.ndkVersion|"[^"]+")'),
    'ndkVersion = "29.0.13113456"',
  );
  value = value.replaceFirst(
    RegExp(r'minSdk\s*=\s*(?:flutter\.minSdkVersion|\d+)'),
    'minSdk = 28',
  );
  value = value.replaceAll('JavaVersion.VERSION_1_8', 'JavaVersion.VERSION_17');
  value = value.replaceAll('JavaVersion.VERSION_11', 'JavaVersion.VERSION_17');

  if (!value.contains('isCoreLibraryDesugaringEnabled = true')) {
    value = value.replaceFirst(
      'compileOptions {',
      'compileOptions {\n        isCoreLibraryDesugaringEnabled = true',
    );
  }
  if (!value.contains('multiDexEnabled = true')) {
    value = value.replaceFirst(
      'defaultConfig {',
      'defaultConfig {\n        multiDexEnabled = true',
    );
  }
  if (!value.contains('coreLibraryDesugaring(')) {
    const dependencies = '''

dependencies {
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.1.4")
}
''';
    if (value.contains('\nflutter {')) {
      value = value.replaceFirst('\nflutter {', '$dependencies\nflutter {');
    } else {
      value = '$value$dependencies';
    }
  }

  await file.writeAsString(value);
}

Future<void> _patchAndroidSettings() async {
  final file = File('android/settings.gradle.kts');
  if (!await file.exists()) {
    throw StateError('Android Gradle settings are missing.');
  }
  var value = await file.readAsString();
  if (!value.contains('com.google.gms.google-services')) {
    value = value.replaceFirst(
      'id("com.android.application")',
      'id("com.google.gms.google-services") version "4.5.0" apply false\n    id("com.android.application")',
    );
  }
  await file.writeAsString(value);
}

Future<void> _patchAndroidMainActivity() async {
  final newDirectory = Directory(
    'android/app/src/main/kotlin/com/openmindai/mobile',
  );
  await newDirectory.create(recursive: true);
  final newFile = File('${newDirectory.path}/MainActivity.kt');
  await newFile.writeAsString('''package $_appId

import io.flutter.embedding.android.FlutterActivity

class MainActivity : FlutterActivity()
''');

  final oldFile = File(
    'android/app/src/main/kotlin/com/openmindai/openmindai_mobile/MainActivity.kt',
  );
  if (await oldFile.exists()) await oldFile.delete();
}

Future<void> _writeAndroidNotificationIcon() async {
  final directory = Directory('android/app/src/main/res/drawable');
  await directory.create(recursive: true);
  final file = File('${directory.path}/openmindai_notification.xml');
  if (await file.exists()) return;
  await file.writeAsString('''<?xml version="1.0" encoding="utf-8"?>
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="24dp"
    android:height="24dp"
    android:viewportWidth="24"
    android:viewportHeight="24">
    <path
        android:fillColor="#FFFFFFFF"
        android:pathData="M12,2A10,10 0,1 0,12 22A10,10 0,0 0,12 2M7,12A5,5 0,0 1,17 12A5,5 0,0 1,7 12M12,8A4,4 0,1 0,12 16A4,4 0,0 0,12 8" />
</vector>
''');
}

Future<void> _patchIosInfoPlist() async {
  final file = File('ios/Runner/Info.plist');
  if (!await file.exists()) {
    throw StateError(
      'iOS host is missing. Restore the committed ios/ host before building.',
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

Future<void> _patchIosBundleIdentifiers() async {
  final project = File('ios/Runner.xcodeproj/project.pbxproj');
  if (!await project.exists()) {
    throw StateError('iOS Xcode project configuration is missing.');
  }
  var value = await project.readAsString();
  value = value.replaceAll(
    RegExp(
      r'PRODUCT_BUNDLE_IDENTIFIER = com\.openmindai\.[A-Za-z0-9_]+\.RunnerTests;',
    ),
    'PRODUCT_BUNDLE_IDENTIFIER = $_appId.RunnerTests;',
  );
  value = value.replaceAll(
    RegExp(r'PRODUCT_BUNDLE_IDENTIFIER = com\.openmindai\.[A-Za-z0-9_]+;'),
    'PRODUCT_BUNDLE_IDENTIFIER = $_appId;',
  );
  await project.writeAsString(value);
}

Future<void> _validateFirebaseConfiguration() async {
  await _validateAndroidFirebase();
  await _validateIosFirebase();
  await _validateDartFirebaseOptions();
}

Future<void> _validateAndroidFirebase() async {
  final file = File('android/app/google-services.json');
  if (!await file.exists()) {
    throw StateError(
      'Firebase Android configuration is missing: android/app/google-services.json',
    );
  }

  final decoded = jsonDecode(await file.readAsString());
  if (decoded is! Map<String, dynamic>) {
    throw StateError('Firebase Android configuration is not valid JSON.');
  }

  final projectInfo = decoded['project_info'];
  if (projectInfo is! Map<String, dynamic> ||
      projectInfo['project_id'] != _firebaseProjectId) {
    throw StateError(
      'Firebase Android project ID does not match $_firebaseProjectId.',
    );
  }

  final clients = decoded['client'];
  if (clients is! List) {
    throw StateError('Firebase Android configuration has no client list.');
  }

  Map<String, dynamic>? matchingClient;
  for (final value in clients) {
    if (value is! Map<String, dynamic>) continue;
    final clientInfo = value['client_info'];
    if (clientInfo is! Map<String, dynamic>) continue;
    final androidInfo = clientInfo['android_client_info'];
    if (androidInfo is Map<String, dynamic> &&
        androidInfo['package_name'] == _appId) {
      matchingClient = value;
      break;
    }
  }

  if (matchingClient == null) {
    throw StateError(
      'Firebase Android configuration has no client for $_appId.',
    );
  }

  final clientInfo = matchingClient['client_info'];
  if (clientInfo is! Map<String, dynamic> ||
      clientInfo['mobilesdk_app_id'] != _androidFirebaseAppId) {
    throw StateError('Firebase Android app ID does not match this app.');
  }
}

Future<void> _validateIosFirebase() async {
  final file = File('ios/Runner/GoogleService-Info.plist');
  if (!await file.exists()) {
    throw StateError(
      'Firebase iOS configuration is missing: ios/Runner/GoogleService-Info.plist',
    );
  }

  final value = await file.readAsString();
  _requirePlistValue(value, 'BUNDLE_ID', _appId);
  _requirePlistValue(value, 'PROJECT_ID', _firebaseProjectId);
  _requirePlistValue(value, 'GOOGLE_APP_ID', _iosFirebaseAppId);
}

void _requirePlistValue(String plist, String key, String expected) {
  final expression = RegExp(
    '<key>${RegExp.escape(key)}</key>\\s*<string>${RegExp.escape(expected)}</string>',
  );
  if (!expression.hasMatch(plist)) {
    throw StateError(
      'Firebase iOS $key does not match the expected OpenMindAI configuration.',
    );
  }
}

Future<void> _validateDartFirebaseOptions() async {
  final file = File('lib/firebase_options.dart');
  if (!await file.exists()) {
    throw StateError('lib/firebase_options.dart is missing.');
  }

  final value = await file.readAsString();
  final requiredValues = <String>[
    "projectId: '$_firebaseProjectId'",
    "appId: '$_androidFirebaseAppId'",
    "appId: '$_iosFirebaseAppId'",
    "iosBundleId: '$_appId'",
  ];
  for (final requiredValue in requiredValues) {
    if (!value.contains(requiredValue)) {
      throw StateError(
        'lib/firebase_options.dart does not match the committed Firebase configuration.',
      );
    }
  }
}
