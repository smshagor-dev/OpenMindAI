import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../core/services/device_profile_service.dart';
import '../../core/services/model_storage_service.dart';
import '../../core/services/permission_service.dart';
import '../../core/storage/onboarding_store.dart';
import '../../core/theme/app_theme.dart';
import '../../core/theme/openmind_ui.dart';

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
            TextButton(
              onPressed: () => Navigator.pop(dialogContext),
              child: const Text('Continue'),
            ),
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
      _WelcomePage(
        loading: _requestingPermissions,
        onContinue: _continueFromWelcome,
      ),
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
              padding: const EdgeInsets.fromLTRB(14, 10, 14, 2),
              child: Row(
                children: [
                  SizedBox(
                    width: 48,
                    child: _step == 0
                        ? null
                        : IconButton(
                            tooltip: 'Back',
                            onPressed: _back,
                            icon: const Icon(Icons.arrow_back_rounded),
                          ),
                  ),
                  Expanded(child: _ProgressDots(active: _step)),
                  SizedBox(
                    width: 48,
                    child: Center(
                      child: Text(
                        '${_step + 1}/4',
                        style: Theme.of(context).textTheme.labelMedium?.copyWith(
                              color: Theme.of(context).colorScheme.onSurfaceVariant,
                            ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
            Expanded(
              child: AnimatedSwitcher(
                duration: const Duration(milliseconds: 220),
                switchInCurve: Curves.easeOut,
                switchOutCurve: Curves.easeIn,
                child: KeyedSubtree(key: ValueKey(_step), child: pages[_step]),
              ),
            ),
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
        final complete = index < active;
        return AnimatedContainer(
          duration: const Duration(milliseconds: 180),
          width: selected ? 28 : 9,
          height: 7,
          margin: const EdgeInsets.symmetric(horizontal: 3),
          decoration: BoxDecoration(
            color: selected || complete
                ? AppTheme.accent
                : Theme.of(context).colorScheme.outlineVariant,
            borderRadius: BorderRadius.circular(99),
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
  Widget build(BuildContext context) {
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 620),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(22, 14, 22, 24),
          child: child,
        ),
      ),
    );
  }
}

class _WelcomePage extends StatelessWidget {
  const _WelcomePage({required this.loading, required this.onContinue});

  final bool loading;
  final VoidCallback onContinue;

  @override
  Widget build(BuildContext context) {
    return _PageShell(
      child: SingleChildScrollView(
        child: Column(
          children: [
            const SizedBox(height: 22),
            const OpenMindBrandMark(size: 84),
            const SizedBox(height: 24),
            Text(
              'Your AI. Your device.',
              textAlign: TextAlign.center,
              style: Theme.of(context).textTheme.headlineLarge,
            ),
            const SizedBox(height: 10),
            Text(
              'OpenMindAI brings private, local-first chat, vision and voice to your phone. You decide when the app uses files, camera, microphone or the web.',
              textAlign: TextAlign.center,
              style: Theme.of(context).textTheme.bodyLarge?.copyWith(
                    color: Theme.of(context).colorScheme.onSurfaceVariant,
                  ),
            ),
            const SizedBox(height: 28),
            const OpenMindSectionCard(
              child: Column(
                children: [
                  _Capability(
                    icon: Icons.camera_alt_outlined,
                    title: 'Camera & photos',
                    text: 'Image and document understanding',
                  ),
                  Divider(height: 20),
                  _Capability(
                    icon: Icons.mic_none_rounded,
                    title: 'OpenMindAI Hear',
                    text: 'Local voice dictation',
                  ),
                  Divider(height: 20),
                  _Capability(
                    icon: Icons.folder_open_rounded,
                    title: 'Files',
                    text: 'Attach PDFs, text and code explicitly',
                  ),
                  Divider(height: 20),
                  _Capability(
                    icon: Icons.notifications_none_rounded,
                    title: 'Notifications',
                    text: 'Optional completion updates',
                  ),
                ],
              ),
            ),
            const SizedBox(height: 26),
            SizedBox(
              width: double.infinity,
              child: FilledButton.icon(
                onPressed: loading ? null : onContinue,
                icon: loading
                    ? const SizedBox(
                        width: 19,
                        height: 19,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.arrow_forward_rounded),
                label: Text(loading ? 'Checking permissions…' : 'Continue'),
              ),
            ),
            const SizedBox(height: 10),
            Text(
              'Only feature-specific permissions are requested. Broad storage permission is not required.',
              textAlign: TextAlign.center,
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ],
        ),
      ),
    );
  }
}

class _Capability extends StatelessWidget {
  const _Capability({required this.icon, required this.title, required this.text});

  final IconData icon;
  final String title;
  final String text;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        OpenMindFeatureIcon(icon),
        const SizedBox(width: 13),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(title, style: const TextStyle(fontWeight: FontWeight.w800)),
              const SizedBox(height: 2),
              Text(
                text,
                style: Theme.of(context).textTheme.bodySmall?.copyWith(
                      color: Theme.of(context).colorScheme.onSurfaceVariant,
                    ),
              ),
            ],
          ),
        ),
        const Icon(Icons.check_circle_outline_rounded, color: AppTheme.accent, size: 20),
      ],
    );
  }
}

class _InstructionsPage extends StatelessWidget {
  const _InstructionsPage({required this.onContinue});

  final VoidCallback onContinue;

  @override
  Widget build(BuildContext context) {
    const items = [
      (
        Icons.storage_rounded,
        'Models use device storage',
        'OpenMindAI recommends a local model from available RAM and free space. Larger models remain optional.',
      ),
      (
        Icons.battery_charging_full_rounded,
        'Local AI uses battery and memory',
        'Long responses and larger models can warm the phone. Leave enough memory and battery for smooth use.',
      ),
      (
        Icons.offline_bolt_outlined,
        'Core chat works offline',
        'After a model is installed, standard chat does not need a paid AI subscription. Search still needs internet.',
      ),
      (
        Icons.tune_rounded,
        'You stay in control',
        'Camera, microphone, files, downloads and network features happen only through explicit app actions.',
      ),
    ];

    return _PageShell(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 8),
          const OpenMindPageHeader(
            title: 'Before you start',
            subtitle: 'Four things to know about running AI directly on a phone.',
          ),
          const SizedBox(height: 22),
          Expanded(
            child: ListView.separated(
              itemCount: items.length,
              separatorBuilder: (_, _) => const SizedBox(height: 10),
              itemBuilder: (context, index) {
                final item = items[index];
                return OpenMindSectionCard(
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      OpenMindFeatureIcon(item.$1),
                      const SizedBox(width: 13),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(item.$2, style: const TextStyle(fontWeight: FontWeight.w800)),
                            const SizedBox(height: 5),
                            Text(
                              item.$3,
                              style: TextStyle(
                                color: Theme.of(context).colorScheme.onSurfaceVariant,
                              ),
                            ),
                          ],
                        ),
                      ),
                    ],
                  ),
                );
              },
            ),
          ),
          const SizedBox(height: 14),
          SizedBox(
            width: double.infinity,
            child: FilledButton(
              onPressed: onContinue,
              child: const Text('I understand'),
            ),
          ),
        ],
      ),
    );
  }
}

class _LicensePage extends StatelessWidget {
  const _LicensePage({
    required this.accepted,
    required this.onChanged,
    required this.onContinue,
  });

  final bool accepted;
  final ValueChanged<bool> onChanged;
  final VoidCallback? onContinue;

  @override
  Widget build(BuildContext context) {
    return _PageShell(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 8),
          const OpenMindPageHeader(
            title: 'License agreement',
            subtitle: 'Review the OpenMindAI license before installing a model.',
          ),
          const SizedBox(height: 16),
          Expanded(
            child: Card(
              child: ClipRRect(
                borderRadius: BorderRadius.circular(20),
                child: FutureBuilder<String>(
                  future: rootBundle.loadString('assets/LICENSE.txt'),
                  builder: (context, snapshot) {
                    if (!snapshot.hasData) {
                      return const Center(child: CircularProgressIndicator(strokeWidth: 2));
                    }
                    return Scrollbar(
                      child: SingleChildScrollView(
                        padding: const EdgeInsets.all(18),
                        child: SelectableText(
                          snapshot.data!,
                          style: const TextStyle(fontSize: 12.5, height: 1.5),
                        ),
                      ),
                    );
                  },
                ),
              ),
            ),
          ),
          const SizedBox(height: 8),
          CheckboxListTile(
            contentPadding: EdgeInsets.zero,
            value: accepted,
            onChanged: (value) => onChanged(value ?? false),
            title: const Text(
              'I have read and agree to the license terms.',
              style: TextStyle(fontWeight: FontWeight.w700),
            ),
            controlAffinity: ListTileControlAffinity.leading,
          ),
          SizedBox(
            width: double.infinity,
            child: FilledButton(
              onPressed: onContinue,
              child: const Text('Agree and continue'),
            ),
          ),
        ],
      ),
    );
  }
}

class _RecommendationPage extends StatefulWidget {
  const _RecommendationPage({
    required this.profileFuture,
    required this.storage,
    required this.onReady,
  });

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
            return const OpenMindEmptyState(
              icon: Icons.memory_rounded,
              title: 'Checking this device',
              description: 'Reading available memory and storage to recommend a balanced local model.',
              action: SizedBox(
                width: 24,
                height: 24,
                child: CircularProgressIndicator(strokeWidth: 2),
              ),
            );
          }
          if (!snapshot.hasData) {
            return OpenMindEmptyState(
              icon: Icons.error_outline_rounded,
              title: 'Device check unavailable',
              description: 'OpenMindAI could not read device capabilities. ${snapshot.error ?? ''}',
            );
          }

          final profile = snapshot.data!;
          final model = profile.recommendedModel;
          final progress = _progress;
          return Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const SizedBox(height: 8),
              OpenMindPageHeader(
                title: 'Recommended model',
                subtitle: '${profile.deviceName} · ${profile.platform} ${profile.osVersion}',
              ),
              const SizedBox(height: 18),
              OpenMindSectionCard(
                padding: const EdgeInsets.all(18),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        const OpenMindFeatureIcon(Icons.auto_awesome_rounded),
                        const SizedBox(width: 12),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              const OpenMindStatusPill(label: 'Recommended', active: true),
                              const SizedBox(height: 7),
                              Text(model.name, style: Theme.of(context).textTheme.titleLarge),
                            ],
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 12),
                    Text(model.description),
                    const SizedBox(height: 16),
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        OpenMindStatusPill(
                          label: '${profile.ramGb} GB RAM',
                          icon: Icons.memory_rounded,
                        ),
                        OpenMindStatusPill(
                          label: '${profile.freeDiskGb.toStringAsFixed(1)} GB free',
                          icon: Icons.storage_rounded,
                        ),
                        OpenMindStatusPill(
                          label: '~${model.sizeGb.toStringAsFixed(1)} GB download',
                          icon: Icons.download_rounded,
                        ),
                      ],
                    ),
                    if (progress != null) ...[
                      const SizedBox(height: 18),
                      LinearProgressIndicator(
                        value: progress.progress > 0 ? progress.progress : null,
                        minHeight: 6,
                        borderRadius: BorderRadius.circular(99),
                      ),
                      const SizedBox(height: 7),
                      Text(progress.stage),
                    ],
                  ],
                ),
              ),
              if (_error != null) ...[
                const SizedBox(height: 12),
                Material(
                  color: Theme.of(context).colorScheme.errorContainer,
                  borderRadius: BorderRadius.circular(16),
                  child: Padding(
                    padding: const EdgeInsets.all(12),
                    child: Row(
                      children: [
                        const Icon(Icons.error_outline_rounded),
                        const SizedBox(width: 9),
                        Expanded(child: Text(_error!)),
                      ],
                    ),
                  ),
                ),
              ],
              const Spacer(),
              SizedBox(
                width: double.infinity,
                child: FilledButton.icon(
                  onPressed: _installing ? null : () => _install(profile),
                  icon: Icon(_installing ? Icons.downloading_rounded : Icons.download_rounded),
                  label: Text(_installing ? 'Installing local model…' : 'Install and open chat'),
                ),
              ),
              if (_installing)
                Center(
                  child: TextButton(
                    onPressed: () => widget.storage.cancelInstall(model.id),
                    child: const Text('Cancel download'),
                  ),
                ),
              const SizedBox(height: 7),
              Text(
                'The model is stored in app-private storage and verified before use.',
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
