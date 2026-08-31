import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../core/services/device_profile_service.dart';
import '../../core/services/model_storage_service.dart';
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
  final _modelStorage = ModelStorageService();

  int _step = 0;
  bool _requestingPermissions = false;
  bool _licenseAccepted = false;
  late final Future<MobileDeviceProfile> _profileFuture;

  @override
  void initState() {
    super.initState();
    _profileFuture = _deviceProfile.read();
  }

  void _next() {
    if (_step < 3) setState(() => _step += 1);
  }

  void _back() {
    if (_step > 0) setState(() => _step -= 1);
  }

  Future<void> _continueFromWelcome() async {
    if (_requestingPermissions) return;
    setState(() => _requestingPermissions = true);
    final result = await _permissions.requestInitialPermissions();
    if (!mounted) return;
    setState(() => _requestingPermissions = false);

    if (result.hasPermanentDenial) {
      await showDialog<void>(
        context: context,
        builder: (dialogContext) => AlertDialog(
          title: const Text('Permission blocked'),
          content: const Text(
            'One or more optional capabilities were permanently denied. Text chat still works and you can enable them later in system settings.',
          ),
          actions: [
            TextButton(onPressed: () => Navigator.pop(dialogContext), child: const Text('Continue')),
            FilledButton(
              onPressed: () {
                Navigator.pop(dialogContext);
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
    final pages = <Widget>[
      _WelcomePage(loading: _requestingPermissions, onContinue: _continueFromWelcome),
      _InstructionsPage(onContinue: _next),
      _LicensePage(
        accepted: _licenseAccepted,
        onChanged: (value) => setState(() => _licenseAccepted = value),
        onContinue: _licenseAccepted ? _next : null,
      ),
      _RecommendationPage(
        profileFuture: _profileFuture,
        storage: _modelStorage,
        onReady: (profile) async {
          await _store.complete(selectedModelId: profile.recommendedModel.id);
          if (mounted) widget.onFinished();
        },
      ),
    ];

    return Scaffold(
      body: SafeArea(
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 10, 16, 2),
              child: Row(
                children: [
                  SizedBox(
                    width: 48,
                    child: _step == 0 ? null : IconButton(onPressed: _back, icon: const Icon(Icons.arrow_back_rounded)),
                  ),
                  Expanded(child: _ProgressDots(active: _step)),
                  const SizedBox(width: 48),
                ],
              ),
            ),
            Expanded(child: pages[_step]),
          ],
        ),
      ),
    );
  }
}

class _ProgressDots extends StatelessWidget {
  const _ProgressDots({required this.active});
  final int active;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisAlignment: MainAxisAlignment.center,
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

class _PageShell extends StatelessWidget {
  const _PageShell({required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.fromLTRB(24, 14, 24, 24),
        child: child,
      );
}

class _WelcomePage extends StatelessWidget {
  const _WelcomePage({required this.loading, required this.onContinue});
  final bool loading;
  final VoidCallback onContinue;

  @override
  Widget build(BuildContext context) {
    return _PageShell(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Container(
            width: 88,
            height: 88,
            decoration: BoxDecoration(color: Theme.of(context).colorScheme.onSurface, shape: BoxShape.circle),
            child: Icon(Icons.psychology_alt_rounded, size: 46, color: Theme.of(context).colorScheme.surface),
          ),
          const SizedBox(height: 28),
          Text(
            'Welcome to OpenMindAI',
            textAlign: TextAlign.center,
            style: Theme.of(context).textTheme.headlineMedium?.copyWith(fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 12),
          Text(
            'Private, local-first AI on your phone. Models and conversations stay under your control.',
            textAlign: TextAlign.center,
            style: Theme.of(context).textTheme.bodyLarge?.copyWith(height: 1.45),
          ),
          const SizedBox(height: 28),
          const _Capability(icon: Icons.camera_alt_outlined, text: 'Camera for image and document input'),
          const _Capability(icon: Icons.mic_none_rounded, text: 'Microphone for voice input'),
          const _Capability(icon: Icons.notifications_none_rounded, text: 'Notifications for completed tasks'),
          const _Capability(icon: Icons.folder_open_rounded, text: 'Files through the system picker'),
          const SizedBox(height: 34),
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
          const SizedBox(height: 10),
          Text(
            'Only permissions used by app features are requested. Broad storage permission is not required.',
            textAlign: TextAlign.center,
            style: Theme.of(context).textTheme.bodySmall,
          ),
        ],
      ),
    );
  }
}

class _Capability extends StatelessWidget {
  const _Capability({required this.icon, required this.text});
  final IconData icon;
  final String text;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.symmetric(vertical: 7),
        child: Row(children: [Icon(icon, size: 21), const SizedBox(width: 14), Expanded(child: Text(text))]),
      );
}

class _InstructionsPage extends StatelessWidget {
  const _InstructionsPage({required this.onContinue});
  final VoidCallback onContinue;

  @override
  Widget build(BuildContext context) {
    const items = [
      ('Models use device storage', 'A suitable local model is recommended from available RAM and storage. Larger models stay optional.'),
      ('Local AI uses battery and memory', 'Long responses and large models can warm the phone. Keep enough free memory and battery.'),
      ('Core chat works offline', 'After a model is installed, normal local chat does not require a paid AI subscription. Search still needs internet.'),
      ('You stay in control', 'Camera, microphone, files, downloads, and network features are explicit app actions.'),
    ];
    return _PageShell(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('Before you start', style: Theme.of(context).textTheme.headlineMedium?.copyWith(fontWeight: FontWeight.w700)),
          const SizedBox(height: 8),
          const Text('A few things make local AI work better on mobile.'),
          const SizedBox(height: 26),
          ...List.generate(items.length, (index) {
            final item = items[index];
            return Padding(
              padding: const EdgeInsets.only(bottom: 20),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  CircleAvatar(radius: 16, child: Text('${index + 1}')),
                  const SizedBox(width: 14),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(item.$1, style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w700)),
                        const SizedBox(height: 5),
                        Text(item.$2, style: TextStyle(height: 1.4, color: Theme.of(context).colorScheme.onSurfaceVariant)),
                      ],
                    ),
                  ),
                ],
              ),
            );
          }),
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

class _LicensePage extends StatelessWidget {
  const _LicensePage({required this.accepted, required this.onChanged, required this.onContinue});
  final bool accepted;
  final ValueChanged<bool> onChanged;
  final VoidCallback? onContinue;

  @override
  Widget build(BuildContext context) {
    return _PageShell(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('License agreement', style: Theme.of(context).textTheme.headlineMedium?.copyWith(fontWeight: FontWeight.w700)),
          const SizedBox(height: 8),
          const Text('Read the OpenMindAI license before continuing.'),
          const SizedBox(height: 16),
          Expanded(
            child: Container(
              width: double.infinity,
              padding: const EdgeInsets.all(16),
              decoration: BoxDecoration(border: Border.all(color: Theme.of(context).dividerColor), borderRadius: BorderRadius.circular(18)),
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

class _RecommendationPage extends StatefulWidget {
  const _RecommendationPage({required this.profileFuture, required this.storage, required this.onReady});
  final Future<MobileDeviceProfile> profileFuture;
  final ModelStorageService storage;
  final ValueChanged<MobileDeviceProfile> onReady;

  @override
  State<_RecommendationPage> createState() => _RecommendationPageState();
}

class _RecommendationPageState extends State<_RecommendationPage> {
  ModelInstallProgress? _progress;
  bool _installing = false;
  String? _error;

  Future<void> _install(MobileDeviceProfile profile) async {
    if (_installing) return;
    setState(() {
      _installing = true;
      _error = null;
    });
    try {
      await widget.storage.install(
        profile.recommendedModel,
        onProgress: (value) {
          if (mounted) setState(() => _progress = value);
        },
      );
      if (mounted) widget.onReady(profile);
    } catch (error) {
      if (mounted) setState(() => _error = error.toString());
    } finally {
      if (mounted) setState(() => _installing = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return _PageShell(
      child: FutureBuilder<MobileDeviceProfile>(
        future: widget.profileFuture,
        builder: (context, snapshot) {
          if (snapshot.connectionState != ConnectionState.done) {
            return const Center(child: Column(mainAxisSize: MainAxisSize.min, children: [CircularProgressIndicator(strokeWidth: 2), SizedBox(height: 16), Text('Checking this device…')]));
          }
          if (!snapshot.hasData) {
            return Center(child: Text('Could not read device capabilities.\n${snapshot.error}', textAlign: TextAlign.center));
          }

          final profile = snapshot.data!;
          final model = profile.recommendedModel;
          final progress = _progress;
          return Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text('Recommended for this device', style: Theme.of(context).textTheme.headlineMedium?.copyWith(fontWeight: FontWeight.w700)),
              const SizedBox(height: 8),
              Text('${profile.deviceName} · ${profile.platform} ${profile.osVersion}'),
              const SizedBox(height: 26),
              Container(
                width: double.infinity,
                padding: const EdgeInsets.all(22),
                decoration: BoxDecoration(color: Theme.of(context).colorScheme.surfaceContainerHighest, borderRadius: BorderRadius.circular(24)),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(children: [
                      const Icon(Icons.auto_awesome_rounded),
                      const SizedBox(width: 10),
                      Expanded(child: Text(model.name, style: Theme.of(context).textTheme.titleLarge?.copyWith(fontWeight: FontWeight.w700))),
                    ]),
                    const SizedBox(height: 12),
                    Text(model.description, style: const TextStyle(height: 1.45)),
                    const SizedBox(height: 16),
                    Wrap(spacing: 8, runSpacing: 8, children: [
                      Chip(label: Text('${profile.ramGb} GB RAM')),
                      Chip(label: Text('${profile.freeDiskGb.toStringAsFixed(1)} GB free')),
                      Chip(label: Text('~${model.sizeGb.toStringAsFixed(1)} GB download')),
                    ]),
                    if (progress != null) ...[
                      const SizedBox(height: 16),
                      LinearProgressIndicator(value: progress.progress > 0 ? progress.progress : null),
                      const SizedBox(height: 7),
                      Text(progress.stage),
                    ],
                  ],
                ),
              ),
              if (_error != null)
                Padding(
                  padding: const EdgeInsets.only(top: 14),
                  child: Text(_error!, style: TextStyle(color: Theme.of(context).colorScheme.error)),
                ),
              const Spacer(),
              SizedBox(
                width: double.infinity,
                child: FilledButton(
                  onPressed: _installing ? null : () => _install(profile),
                  style: FilledButton.styleFrom(padding: const EdgeInsets.symmetric(vertical: 16)),
                  child: _installing ? const Text('Installing local model…') : const Text('Install and open chat'),
                ),
              ),
              if (_installing)
                Center(
                  child: TextButton(
                    onPressed: () => widget.storage.cancelInstall(model.id),
                    child: const Text('Cancel download'),
                  ),
                ),
              const SizedBox(height: 6),
              Text(
                'The model is downloaded to app-private storage and verified before use. Only the OpenMindAI product name is shown in the app.',
                textAlign: TextAlign.center,
                style: Theme.of(context).textTheme.bodySmall,
              ),
            ],
          );
        },
      ),
    );
  }
}
