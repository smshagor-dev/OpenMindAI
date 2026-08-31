import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../../core/services/device_profile_service.dart';
import '../../core/services/permission_service.dart';
import '../../core/storage/onboarding_store.dart';

class OnboardingFlow extends StatefulWidget {
  const OnboardingFlow({super.key, required this.onFinished});

  final VoidCallback onFinished;

  @override
  State<OnboardingFlow> createState() => _OnboardingFlowState();
}

class _OnboardingFlowState extends State<OnboardingFlow> {
  final _permissions = PermissionService();
  final _store = OnboardingStore();
  final _deviceProfile = DeviceProfileService();
  int _step = 0;
  bool _requestingPermissions = false;
  bool _licenseAccepted = false;
  late final Future<MobileDeviceProfile> _profileFuture;

  @override
  void initState() {
    super.initState();
    _profileFuture = _deviceProfile.read();
  }

  void _next() => setState(() => _step = (_step + 1).clamp(0, 3));
  void _back() => setState(() => _step = (_step - 1).clamp(0, 3));

  Future<void> _continueFromWelcome() async {
    if (_requestingPermissions) return;
    setState(() => _requestingPermissions = true);
    final result = await _permissions.requestInitialPermissions();
    if (!mounted) return;
    setState(() => _requestingPermissions = false);
    if (result.hasPermanentDenial) {
      await showDialog<void>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('Permission blocked'),
          content: const Text(
            'One or more permissions were permanently denied. You can enable them later in system settings; core text chat can still continue.',
          ),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context), child: const Text('Continue')),
            FilledButton(
              onPressed: () {
                Navigator.pop(context);
                _permissions.openSettings();
              },
              child: const Text('Open settings'),
            ),
          ],
        ),
      );
    }
    if (mounted) _next();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SafeArea(
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(20, 12, 20, 4),
              child: Row(
                children: [
                  if (_step > 0)
                    IconButton(onPressed: _back, icon: const Icon(Icons.arrow_back))
                  else
                    const SizedBox(width: 48),
                  const Spacer(),
                  _StepDots(active: _step),
                  const Spacer(),
                  const SizedBox(width: 48),
                ],
              ),
            ),
            Expanded(
              child: IndexedStack(
                index: _step,
                children: [
                  _WelcomeScreen(
                    loading: _requestingPermissions,
                    onContinue: _continueFromWelcome,
                  ),
                  _InstructionsScreen(onContinue: _next),
                  _LicenseScreen(
                    accepted: _licenseAccepted,
                    onChanged: (value) => setState(() => _licenseAccepted = value),
                    onContinue: _licenseAccepted ? _next : null,
                  ),
                  _RecommendationScreen(
                    profileFuture: _profileFuture,
                    onFinish: (profile) async {
                      await _store.complete(selectedModelId: profile.recommendedModel.id);
                      if (mounted) widget.onFinished();
                    },
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _StepDots extends StatelessWidget {
  const _StepDots({required this.active});
  final int active;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: List.generate(4, (index) {
        final selected = index == active;
        return AnimatedContainer(
          duration: const Duration(milliseconds: 180),
          width: selected ? 22 : 7,
          height: 7,
          margin: const EdgeInsets.symmetric(horizontal: 3),
          decoration: BoxDecoration(
            color: selected
                ? Theme.of(context).colorScheme.onSurface
                : Theme.of(context).colorScheme.onSurface.withValues(alpha: .2),
            borderRadius: BorderRadius.circular(10),
          ),
        );
      }),
    );
  }
}

class _WelcomeScreen extends StatelessWidget {
  const _WelcomeScreen({required this.loading, required this.onContinue});
  final bool loading;
  final VoidCallback onContinue;

  @override
  Widget build(BuildContext context) {
    return _OnboardingPage(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Container(
            width: 88,
            height: 88,
            decoration: BoxDecoration(
              color: Theme.of(context).colorScheme.onSurface,
              shape: BoxShape.circle,
            ),
            child: Icon(
              Icons.psychology_alt_rounded,
              size: 46,
              color: Theme.of(context).colorScheme.surface,
            ),
          ),
          const SizedBox(height: 30),
          Text('Welcome to OpenMindAI', style: Theme.of(context).textTheme.headlineMedium?.copyWith(fontWeight: FontWeight.w700)),
          const SizedBox(height: 12),
          Text(
            'Private, local-first AI on your phone. Your core model and conversations stay on the device.',
            textAlign: TextAlign.center,
            style: Theme.of(context).textTheme.bodyLarge?.copyWith(height: 1.45),
          ),
          const SizedBox(height: 30),
          const _PermissionLine(icon: Icons.camera_alt_outlined, text: 'Camera for image and document input'),
          const _PermissionLine(icon: Icons.mic_none_rounded, text: 'Microphone for voice input'),
          const _PermissionLine(icon: Icons.notifications_none_rounded, text: 'Notifications for completed local tasks'),
          const _PermissionLine(icon: Icons.folder_open_rounded, text: 'Files through the system file picker'),
          const SizedBox(height: 36),
          SizedBox(
            width: double.infinity,
            child: FilledButton(
              onPressed: loading ? null : onContinue,
              style: FilledButton.styleFrom(padding: const EdgeInsets.symmetric(vertical: 16)),
              child: loading
                  ? const SizedBox(width: 20, height: 20, child: CircularProgressIndicator(strokeWidth: 2))
                  : const Text('Continue'),
            ),
          ),
          const SizedBox(height: 12),
          Text(
            'Only permissions used by app features are requested. Broad storage access is not required.',
            textAlign: TextAlign.center,
            style: Theme.of(context).textTheme.bodySmall,
          ),
        ],
      ),
    );
  }
}

class _PermissionLine extends StatelessWidget {
  const _PermissionLine({required this.icon, required this.text});
  final IconData icon;
  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 7),
      child: Row(children: [Icon(icon, size: 21), const SizedBox(width: 14), Expanded(child: Text(text))]),
    );
  }
}

class _InstructionsScreen extends StatelessWidget {
  const _InstructionsScreen({required this.onContinue});
  final VoidCallback onContinue;

  @override
  Widget build(BuildContext context) {
    return _OnboardingPage(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('Before you start', style: Theme.of(context).textTheme.headlineMedium?.copyWith(fontWeight: FontWeight.w700)),
          const SizedBox(height: 10),
          Text('A few things make local AI work better on mobile.', style: Theme.of(context).textTheme.bodyLarge),
          const SizedBox(height: 28),
          const _InstructionCard(number: '1', title: 'Models use device storage', body: 'OpenMindAI will recommend a model based on your RAM and available storage. Larger models are optional.'),
          const _InstructionCard(number: '2', title: 'Local inference uses battery and memory', body: 'Long responses and larger models can warm the device. Keep enough free memory and battery for best performance.'),
          const _InstructionCard(number: '3', title: 'Internet is optional for core chat', body: 'Downloaded local models can answer without a cloud AI subscription. Web Search and connected services require internet.'),
          const _InstructionCard(number: '4', title: 'You stay in control', body: 'Camera, microphone, files, model downloads, and connected services remain explicit user actions.'),
          const Spacer(),
          SizedBox(
            width: double.infinity,
            child: FilledButton(
              onPressed: onContinue,
              style: FilledButton.styleFrom(padding: const EdgeInsets.symmetric(vertical: 16)),
              child: const Text('I understand'),
            ),
          ),
        ],
      ),
    );
  }
}

class _InstructionCard extends StatelessWidget {
  const _InstructionCard({required this.number, required this.title, required this.body});
  final String number;
  final String title;
  final String body;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 20),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          CircleAvatar(radius: 16, child: Text(number)),
          const SizedBox(width: 14),
          Expanded(child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Text(title, style: const TextStyle(fontWeight: FontWeight.w700, fontSize: 16)),
            const SizedBox(height: 5),
            Text(body, style: TextStyle(height: 1.4, color: Theme.of(context).colorScheme.onSurfaceVariant)),
          ])),
        ],
      ),
    );
  }
}

class _LicenseScreen extends StatelessWidget {
  const _LicenseScreen({required this.accepted, required this.onChanged, required this.onContinue});
  final bool accepted;
  final ValueChanged<bool> onChanged;
  final VoidCallback? onContinue;

  @override
  Widget build(BuildContext context) {
    return _OnboardingPage(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('License agreement', style: Theme.of(context).textTheme.headlineMedium?.copyWith(fontWeight: FontWeight.w700)),
          const SizedBox(height: 8),
          const Text('Read the OpenMindAI license before continuing.'),
          const SizedBox(height: 18),
          Expanded(
            child: Container(
              width: double.infinity,
              padding: const EdgeInsets.all(16),
              decoration: BoxDecoration(
                border: Border.all(color: Theme.of(context).dividerColor),
                borderRadius: BorderRadius.circular(18),
              ),
              child: FutureBuilder<String>(
                future: rootBundle.loadString('assets/LICENSE.txt'),
                builder: (context, snapshot) {
                  if (!snapshot.hasData) return const Center(child: CircularProgressIndicator(strokeWidth: 2));
                  return SingleChildScrollView(child: SelectableText(snapshot.data!, style: const TextStyle(fontSize: 12.5, height: 1.45)));
                },
              ),
            ),
          ),
          CheckboxListTile(
            contentPadding: EdgeInsets.zero,
            value: accepted,
            onChanged: (value) => onChanged(value ?? false),
            title: const Text('I have read and agree to the license terms.'),
            controlAffinity: ListTileControlAffinity.leading,
          ),
          SizedBox(
            width: double.infinity,
            child: FilledButton(
              onPressed: onContinue,
              style: FilledButton.styleFrom(padding: const EdgeInsets.symmetric(vertical: 16)),
              child: const Text('Agree and continue'),
            ),
          ),
        ],
      ),
    );
  }
}

class _RecommendationScreen extends StatelessWidget {
  const _RecommendationScreen({required this.profileFuture, required this.onFinish});
  final Future<MobileDeviceProfile> profileFuture;
  final ValueChanged<MobileDeviceProfile> onFinish;

  @override
  Widget build(BuildContext context) {
    return _OnboardingPage(
      child: FutureBuilder<MobileDeviceProfile>(
        future: profileFuture,
        builder: (context, snapshot) {
          if (snapshot.connectionState != ConnectionState.done) {
            return const Center(child: Column(mainAxisSize: MainAxisSize.min, children: [CircularProgressIndicator(strokeWidth: 2), SizedBox(height: 18), Text('Checking this device…')]));
          }
          if (snapshot.hasError || !snapshot.hasData) {
            return Center(child: Column(mainAxisSize: MainAxisSize.min, children: [
              const Icon(Icons.error_outline, size: 42),
              const SizedBox(height: 12),
              Text('Could not read device capabilities.\n${snapshot.error}', textAlign: TextAlign.center),
            ]));
          }
          final profile = snapshot.data!;
          final model = profile.recommendedModel;
          return Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text('Recommended for this device', style: Theme.of(context).textTheme.headlineMedium?.copyWith(fontWeight: FontWeight.w700)),
              const SizedBox(height: 8),
              Text('${profile.deviceName} · ${profile.platform} ${profile.osVersion}'),
              const SizedBox(height: 28),
              Container(
                width: double.infinity,
                padding: const EdgeInsets.all(22),
                decoration: BoxDecoration(
                  color: Theme.of(context).colorScheme.surfaceContainerHighest,
                  borderRadius: BorderRadius.circular(24),
                ),
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Row(children: [
                    const Icon(Icons.auto_awesome_rounded),
                    const SizedBox(width: 10),
                    Expanded(child: Text(model.name, style: Theme.of(context).textTheme.titleLarge?.copyWith(fontWeight: FontWeight.w700))),
                    const Chip(label: Text('Recommended')),
                  ]),
                  const SizedBox(height: 14),
                  Text(model.description, style: const TextStyle(height: 1.45)),
                  const SizedBox(height: 18),
                  Wrap(spacing: 8, runSpacing: 8, children: [
                    Chip(label: Text('${profile.ramGb} GB RAM')),
                    Chip(label: Text('${profile.freeDiskGb.toStringAsFixed(1)} GB free')),
                    Chip(label: Text(model.kind)),
                  ]),
                ]),
              ),
              const SizedBox(height: 18),
              Text(
                'The model name shown in OpenMindAI is the product name used by the desktop app. Technical upstream model names are not shown in the mobile UI.',
                style: Theme.of(context).textTheme.bodySmall,
              ),
              const Spacer(),
              SizedBox(
                width: double.infinity,
                child: FilledButton(
                  onPressed: () => onFinish(profile),
                  style: FilledButton.styleFrom(padding: const EdgeInsets.symmetric(vertical: 16)),
                  child: const Text('Open chat'),
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}

class _OnboardingPage extends StatelessWidget {
  const _OnboardingPage({required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 14, 24, 24),
      child: child,
    );
  }
}
