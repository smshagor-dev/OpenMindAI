import 'package:flutter/material.dart';

import '../core/storage/app_settings_controller.dart';
import '../core/storage/onboarding_store.dart';
import '../core/theme/app_theme.dart';
import '../features/chat/chat_screen.dart';
import '../features/onboarding/onboarding_flow.dart';

class OpenMindMobileApp extends StatefulWidget {
  const OpenMindMobileApp({super.key});

  @override
  State<OpenMindMobileApp> createState() => _OpenMindMobileAppState();
}

class _OpenMindMobileAppState extends State<OpenMindMobileApp> {
  final _store = OnboardingStore();
  final _settings = AppSettingsController.instance;
  bool? _onboardingComplete;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    await _settings.load();
    final complete = await _store.isComplete();
    if (!mounted) return;
    setState(() => _onboardingComplete = complete);
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _settings,
      builder: (context, _) => MaterialApp(
        debugShowCheckedModeBanner: false,
        title: 'OpenMindAI',
        theme: AppTheme.light,
        darkTheme: AppTheme.dark,
        themeMode: _settings.themeMode,
        home: _onboardingComplete == null
            ? const _BootScreen()
            : _onboardingComplete!
                ? const ChatScreen()
                : OnboardingFlow(
                    onFinished: () => setState(() => _onboardingComplete = true),
                  ),
      ),
    );
  }
}

class _BootScreen extends StatelessWidget {
  const _BootScreen();

  @override
  Widget build(BuildContext context) {
    return const Scaffold(
      body: Center(child: CircularProgressIndicator(strokeWidth: 2)),
    );
  }
}
