import 'package:firebase_analytics/firebase_analytics.dart';
import 'package:firebase_core/firebase_core.dart';
import 'package:flutter/foundation.dart';

import '../../firebase_options.dart';

/// Initializes Firebase without making local AI features depend on the network.
///
/// Firebase Analytics only receives Firebase's app/usage telemetry. OpenMindAI
/// prompts, responses, attachments, local model files, and chat history are not
/// sent by this service.
class FirebaseService {
  FirebaseService._();

  static bool _initialized = false;

  static bool get initialized => _initialized;

  static FirebaseAnalytics? get analytics =>
      _initialized ? FirebaseAnalytics.instance : null;

  static Future<void> initialize() async {
    if (_initialized) return;

    try {
      await Firebase.initializeApp(
        options: DefaultFirebaseOptions.currentPlatform,
      );
      await FirebaseAnalytics.instance.setAnalyticsCollectionEnabled(true);
      _initialized = true;
    } catch (error, stackTrace) {
      // Firebase must never prevent the local-first app from opening offline.
      if (kDebugMode) {
        debugPrint('Firebase initialization failed: $error');
        debugPrintStack(stackTrace: stackTrace);
      }
    }
  }
}
