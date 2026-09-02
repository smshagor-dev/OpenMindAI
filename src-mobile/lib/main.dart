import 'dart:async';

import 'package:flutter/material.dart';

import 'app/open_mind_mobile_app.dart';
import 'core/services/firebase_service.dart';
import 'core/services/model_storage_service.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await FirebaseService.initialize();
  unawaited(ModelStorageService().resumeIncompleteInstalls());
  runApp(const OpenMindMobileApp());
}
