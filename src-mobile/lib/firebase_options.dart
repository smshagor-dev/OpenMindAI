import 'package:firebase_core/firebase_core.dart';
import 'package:flutter/foundation.dart'
    show TargetPlatform, defaultTargetPlatform, kIsWeb;

class DefaultFirebaseOptions {
  static FirebaseOptions get currentPlatform {
    if (kIsWeb) return web;
    switch (defaultTargetPlatform) {
      case TargetPlatform.android:
        return android;
      case TargetPlatform.iOS:
        return ios;
      case TargetPlatform.macOS:
      case TargetPlatform.windows:
      case TargetPlatform.linux:
      case TargetPlatform.fuchsia:
        throw UnsupportedError(
          'Firebase is configured only for OpenMindAI web, Android, and iOS.',
        );
    }
  }

  static const FirebaseOptions web = FirebaseOptions(
    apiKey: 'AIzaSyAufUqYHcx4f6L-Y1gwEbfbFguI2HIBFQM',
    appId: '1:369874806080:web:95ee7a3fcbc5fa6e5c86a5',
    messagingSenderId: '369874806080',
    projectId: 'open-mind-ai-b38e2',
    authDomain: 'open-mind-ai-b38e2.firebaseapp.com',
    storageBucket: 'open-mind-ai-b38e2.firebasestorage.app',
    measurementId: 'G-ZCCPMPHXD1',
  );

  static const FirebaseOptions android = FirebaseOptions(
    apiKey: 'AIzaSyCSkdVWl_o9qkRFxmTa3nMm1wC0w0SyVw8',
    appId: '1:369874806080:android:f0749aaa2cc210d95c86a5',
    messagingSenderId: '369874806080',
    projectId: 'open-mind-ai-b38e2',
    storageBucket: 'open-mind-ai-b38e2.firebasestorage.app',
  );

  static const FirebaseOptions ios = FirebaseOptions(
    apiKey: 'AIzaSyDih37tvn6VbOrlSWV6bbph4aiqWGZRdd0',
    appId: '1:369874806080:ios:65c7b85e4f5ff5775c86a5',
    messagingSenderId: '369874806080',
    projectId: 'open-mind-ai-b38e2',
    storageBucket: 'open-mind-ai-b38e2.firebasestorage.app',
    iosBundleId: 'com.openmindai.mobile',
  );
}
