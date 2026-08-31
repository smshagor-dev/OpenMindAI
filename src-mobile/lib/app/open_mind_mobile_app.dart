import 'package:flutter/material.dart';
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
  bool? _onboardingComplete;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final complete = await _store.isComplete();
    if (!mounted) return;
    setState(() => _onboardingComplete = complete);
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      title: 'OpenMindAI',
      theme: AppTheme.light,
      darkTheme: AppTheme.dark,
      themeMode: ThemeMode.system,
      home: _onboardingComplete == null
          ? const _BootScreen()
          : _onboardingComplete!
              ? const ChatScreen()
              : OnboardingFlow(
                  onFinished: () => setState(() => _onboardingComplete = true),
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
