import 'package:flutter/material.dart';

import 'app/open_mind_mobile_app.dart';
import 'core/services/firebase_service.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await FirebaseService.initialize();
  runApp(const OpenMindMobileApp());
}
