import 'dart:io';

Future<void> main() async {
  await _patchAndroidManifest();
  await _patchIosInfoPlist();
  await _patchIosPodfile();
  stdout.writeln('OpenMindAI mobile platform configuration is ready.');
}

Future<void> _patchAndroidManifest() async {
  final file = File('android/app/src/main/AndroidManifest.xml');
  if (!await file.exists()) {
    throw StateError('Android host is missing. Run flutter create --platforms=android,ios . first.');
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
  await file.writeAsString(value);
}

Future<void> _patchIosInfoPlist() async {
  final file = File('ios/Runner/Info.plist');
  if (!await file.exists()) {
    throw StateError('iOS host is missing. Run flutter create --platforms=android,ios . first.');
  }
  var value = await file.readAsString();
  const capabilities = '''
	<key>NSCameraUsageDescription</key>
	<string>OpenMindAI uses the camera only when you choose to attach an image or document.</string>
	<key>NSMicrophoneUsageDescription</key>
	<string>OpenMindAI uses the microphone only when you choose voice input.</string>
	<key>NSPhotoLibraryUsageDescription</key>
	<string>OpenMindAI accesses photos only when you choose images to attach to a conversation.</string>
''';
  if (!value.contains('NSCameraUsageDescription')) {
    value = value.replaceFirst('\n</dict>', '$capabilities\n</dict>');
  }
  await file.writeAsString(value);
}

Future<void> _patchIosPodfile() async {
  final file = File('ios/Podfile');
  if (!await file.exists()) return;
  var value = await file.readAsString();
  if (value.contains("platform :ios, '")) {
    value = value.replaceFirst(RegExp(r"platform :ios, '[^']+'"), "platform :ios, '13.0'");
  } else {
    value = "platform :ios, '13.0'\n$value";
  }
  await file.writeAsString(value);
}
