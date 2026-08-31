import 'package:flutter/material.dart';

import '../../core/services/model_storage_service.dart';
import '../../core/storage/app_settings_controller.dart';
import '../../core/theme/openmind_ui.dart';
import '../models/model_manager_sheet.dart';

class SettingsScreen extends StatefulWidget {
  const SettingsScreen({super.key});

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  final _settings = AppSettingsController.instance;
  final _modelStorage = ModelStorageService();

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _settings,
      builder: (context, _) => Scaffold(
        appBar: AppBar(title: const Text('Settings')),
        body: ListView(
          padding: const EdgeInsets.fromLTRB(18, 8, 18, 32),
          children: [
            const OpenMindPageHeader(
              title: 'Make OpenMindAI yours',
              subtitle: 'Appearance, local data, models and device behavior.',
            ),
            const SizedBox(height: 22),
            _sectionLabel(context, 'Appearance'),
            const SizedBox(height: 8),
            OpenMindSectionCard(
              child: Column(
                children: [
                  _ThemePicker(
                    value: _settings.themeMode,
                    onChanged: _settings.setThemeMode,
                  ),
                  const Divider(height: 26),
                  SwitchListTile.adaptive(
                    contentPadding: EdgeInsets.zero,
                    title: const Text('Compact chat spacing'),
                    subtitle: const Text('Fit more conversation on screen.'),
                    value: _settings.compactChat,
                    onChanged: _settings.setCompactChat,
                  ),
                  SwitchListTile.adaptive(
                    contentPadding: EdgeInsets.zero,
                    title: const Text('Haptic feedback'),
                    subtitle: const Text('Subtle response to important controls.'),
                    value: _settings.haptics,
                    onChanged: _settings.setHaptics,
                  ),
                ],
              ),
            ),
            const SizedBox(height: 22),
            _sectionLabel(context, 'Local AI & storage'),
            const SizedBox(height: 8),
            OpenMindSectionCard(
              child: Column(
                children: [
                  _SettingsRow(
                    icon: Icons.memory_rounded,
                    title: 'Models',
                    subtitle: 'Install, verify or remove local models.',
                    onTap: () => showModelManagerSheet(
                      context,
                      storage: _modelStorage,
                    ),
                  ),
                  const Divider(height: 18),
                  const _SettingsRow(
                    icon: Icons.folder_open_rounded,
                    title: 'App-private storage',
                    subtitle: 'Models and app data stay in the app sandbox.',
                  ),
                  const Divider(height: 18),
                  const _SettingsRow(
                    icon: Icons.shield_outlined,
                    title: 'Privacy',
                    subtitle: 'Core chat runs locally after a model is installed.',
                  ),
                ],
              ),
            ),
            const SizedBox(height: 22),
            _sectionLabel(context, 'Voice & input'),
            const SizedBox(height: 8),
            const OpenMindSectionCard(
              child: Column(
                children: [
                  _SettingsRow(
                    icon: Icons.mic_none_rounded,
                    title: 'OpenMindAI Hear',
                    subtitle: 'Local voice dictation from the chat composer.',
                  ),
                  Divider(height: 18),
                  _SettingsRow(
                    icon: Icons.volume_up_outlined,
                    title: 'OpenMindAI Speak',
                    subtitle: 'Read assistant replies using a device voice.',
                  ),
                  Divider(height: 18),
                  _SettingsRow(
                    icon: Icons.camera_alt_outlined,
                    title: 'Camera & photos',
                    subtitle: 'Images are attached only when you choose them.',
                  ),
                ],
              ),
            ),
            const SizedBox(height: 22),
            _sectionLabel(context, 'About'),
            const SizedBox(height: 8),
            const OpenMindSectionCard(
              child: Column(
                children: [
                  _SettingsRow(
                    icon: Icons.auto_awesome_rounded,
                    title: 'OpenMindAI Mobile',
                    subtitle: 'Private, local-first AI for Android and iOS.',
                  ),
                  Divider(height: 18),
                  _SettingsRow(
                    icon: Icons.info_outline_rounded,
                    title: 'Design system',
                    subtitle: 'OpenMindAI Mobile UI · Material 3.',
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _sectionLabel(BuildContext context, String label) => Padding(
        padding: const EdgeInsets.only(left: 4),
        child: Text(
          label.toUpperCase(),
          style: Theme.of(context).textTheme.labelSmall?.copyWith(
                letterSpacing: 1.1,
                fontWeight: FontWeight.w800,
                color: Theme.of(context).colorScheme.onSurfaceVariant,
              ),
        ),
      );
}

class _ThemePicker extends StatelessWidget {
  const _ThemePicker({required this.value, required this.onChanged});

  final ThemeMode value;
  final ValueChanged<ThemeMode> onChanged;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const Text('Theme', style: TextStyle(fontWeight: FontWeight.w700)),
        const SizedBox(height: 12),
        SegmentedButton<ThemeMode>(
          showSelectedIcon: false,
          segments: const [
            ButtonSegment(
              value: ThemeMode.system,
              icon: Icon(Icons.brightness_auto_rounded),
              label: Text('System'),
            ),
            ButtonSegment(
              value: ThemeMode.light,
              icon: Icon(Icons.light_mode_outlined),
              label: Text('Light'),
            ),
            ButtonSegment(
              value: ThemeMode.dark,
              icon: Icon(Icons.dark_mode_outlined),
              label: Text('Dark'),
            ),
          ],
          selected: {value},
          onSelectionChanged: (values) => onChanged(values.first),
        ),
      ],
    );
  }
}

class _SettingsRow extends StatelessWidget {
  const _SettingsRow({
    required this.icon,
    required this.title,
    required this.subtitle,
    this.onTap,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      contentPadding: EdgeInsets.zero,
      leading: OpenMindFeatureIcon(icon),
      title: Text(title, style: const TextStyle(fontWeight: FontWeight.w700)),
      subtitle: Padding(
        padding: const EdgeInsets.only(top: 3),
        child: Text(subtitle),
      ),
      trailing: onTap == null ? null : const Icon(Icons.chevron_right_rounded),
      onTap: onTap,
    );
  }
}
