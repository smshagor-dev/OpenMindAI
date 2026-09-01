import 'dart:convert';
import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../core/services/attachment_storage_service.dart';
import '../../core/services/model_storage_service.dart';
import '../../core/services/permission_service.dart';
import '../../core/storage/app_settings_controller.dart';
import '../../core/theme/openmind_ui.dart';
import '../chat/services/chat_store.dart';
import '../models/model_manager_sheet.dart';

class SettingsScreen extends StatefulWidget {
  const SettingsScreen({super.key, this.onHistoryChanged});

  final Future<void> Function()? onHistoryChanged;

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  final _settings = AppSettingsController.instance;
  final _modelStorage = ModelStorageService();
  final _chatStore = ChatStore();
  final _attachments = AttachmentStorageService();
  final _permissions = PermissionService();

  late Future<_LocalUsage> _usageFuture;

  @override
  void initState() {
    super.initState();
    _usageFuture = _readUsage();
  }

  Future<_LocalUsage> _readUsage() async {
    final conversations = await _chatStore.load();
    return _LocalUsage(
      conversations: conversations.length,
      databaseBytes: await _chatStore.sizeBytes(),
      attachmentBytes: await _attachments.sizeBytes(),
    );
  }

  void _refreshUsage() {
    if (!mounted) return;
    setState(() => _usageFuture = _readUsage());
  }

  void _haptic() {
    if (_settings.haptics) HapticFeedback.selectionClick();
  }

  Future<void> _exportChats() async {
    _haptic();
    try {
      final json = await _chatStore.exportJson();
      final result = await FilePicker.saveFile(
        dialogTitle: 'Export OpenMindAI chats',
        fileName: 'openmindai-chats-${DateTime.now().millisecondsSinceEpoch}.json',
        bytes: Uint8List.fromList(utf8.encode(json)),
      );
      if (!mounted || result == null) return;
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('Chat export saved.')),
      );
    } catch (error) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Could not export chats: $error')),
      );
    }
  }

  Future<void> _cleanupAttachments() async {
    _haptic();
    final conversations = await _chatStore.load();
    final referenced = <String>{
      for (final conversation in conversations)
        for (final message in conversation.messages) ...message.attachmentPaths,
    };
    final removed = await _attachments.cleanupOrphans(referenced);
    _refreshUsage();
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text('Removed $removed unreferenced attachment file(s).')),
    );
  }

  Future<void> _clearHistory() async {
    _haptic();
    final approved = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('Clear all chats?'),
        content: const Text(
          'This permanently deletes local conversation history and chat attachments. Installed AI models are not removed.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(dialogContext, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(dialogContext, true),
            child: const Text('Clear chats'),
          ),
        ],
      ),
    );
    if (approved != true) return;

    await _chatStore.clear();
    await _attachments.clearAll();
    await widget.onHistoryChanged?.call();
    _refreshUsage();
    if (!mounted) return;
    if (_settings.haptics) HapticFeedback.mediumImpact();
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('Local chat history cleared.')),
    );
  }

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
              subtitle: 'Appearance, interaction, local data, models and device permissions.',
            ),
            const SizedBox(height: 22),
            _sectionLabel(context, 'Appearance'),
            const SizedBox(height: 8),
            OpenMindSectionCard(
              child: Column(
                children: [
                  _ThemePicker(
                    value: _settings.themeMode,
                    onChanged: (value) {
                      _haptic();
                      _settings.setThemeMode(value);
                    },
                  ),
                  const Divider(height: 26),
                  SwitchListTile.adaptive(
                    contentPadding: EdgeInsets.zero,
                    title: const Text('Compact chat spacing'),
                    subtitle: const Text('Fit more conversation on screen.'),
                    value: _settings.compactChat,
                    onChanged: (value) {
                      _haptic();
                      _settings.setCompactChat(value);
                    },
                  ),
                  SwitchListTile.adaptive(
                    contentPadding: EdgeInsets.zero,
                    title: const Text('Haptic feedback'),
                    subtitle: const Text('Use subtle device feedback for important controls.'),
                    value: _settings.haptics,
                    onChanged: _settings.setHaptics,
                  ),
                  SwitchListTile.adaptive(
                    contentPadding: EdgeInsets.zero,
                    title: const Text('Completion notifications'),
                    subtitle: const Text('Notify when a long local response finishes while the app is not active.'),
                    value: _settings.completionNotifications,
                    onChanged: (value) {
                      _haptic();
                      _settings.setCompletionNotifications(value);
                    },
                  ),
                ],
              ),
            ),
            const SizedBox(height: 22),
            _sectionLabel(context, 'Local AI & models'),
            const SizedBox(height: 8),
            OpenMindSectionCard(
              child: Column(
                children: [
                  _SettingsRow(
                    icon: Icons.memory_rounded,
                    title: 'Models',
                    subtitle: 'Install, verify, cancel downloads or remove local models.',
                    onTap: () => showModelManagerSheet(
                      context,
                      storage: _modelStorage,
                    ),
                  ),
                  const Divider(height: 18),
                  const _SettingsRow(
                    icon: Icons.folder_open_rounded,
                    title: 'App-private storage',
                    subtitle: 'Models, SQLite history, Canvas files and attachments stay inside the app sandbox until you export them.',
                  ),
                ],
              ),
            ),
            const SizedBox(height: 22),
            _sectionLabel(context, 'Local data'),
            const SizedBox(height: 8),
            FutureBuilder<_LocalUsage>(
              future: _usageFuture,
              builder: (context, snapshot) {
                final usage = snapshot.data;
                return OpenMindSectionCard(
                  child: Column(
                    children: [
                      _SettingsRow(
                        icon: Icons.chat_bubble_outline_rounded,
                        title: 'Conversation database',
                        subtitle: usage == null
                            ? 'Reading local usage…'
                            : '${usage.conversations} chats · ${_formatBytes(usage.databaseBytes)}',
                      ),
                      const Divider(height: 18),
                      _SettingsRow(
                        icon: Icons.attach_file_rounded,
                        title: 'Saved chat attachments',
                        subtitle: usage == null
                            ? 'Reading local usage…'
                            : _formatBytes(usage.attachmentBytes),
                        onTap: _cleanupAttachments,
                      ),
                      const Divider(height: 18),
                      _SettingsRow(
                        icon: Icons.ios_share_rounded,
                        title: 'Export chats',
                        subtitle: 'Save a portable JSON copy with conversation metadata and attachment paths.',
                        onTap: _exportChats,
                      ),
                      const Divider(height: 18),
                      _SettingsRow(
                        icon: Icons.delete_sweep_outlined,
                        title: 'Clear chat history',
                        subtitle: 'Delete conversations and their app-private attachment copies.',
                        destructive: true,
                        onTap: _clearHistory,
                      ),
                    ],
                  ),
                );
              },
            ),
            const SizedBox(height: 22),
            _sectionLabel(context, 'Voice, camera & permissions'),
            const SizedBox(height: 8),
            OpenMindSectionCard(
              child: Column(
                children: [
                  const _SettingsRow(
                    icon: Icons.mic_none_rounded,
                    title: 'OpenMindAI Hear',
                    subtitle: 'Whisper-powered local voice dictation from the chat composer.',
                  ),
                  const Divider(height: 18),
                  const _SettingsRow(
                    icon: Icons.volume_up_outlined,
                    title: 'OpenMindAI Speak',
                    subtitle: 'Read assistant replies using an installed device voice.',
                  ),
                  const Divider(height: 18),
                  const _SettingsRow(
                    icon: Icons.camera_alt_outlined,
                    title: 'Camera & photos',
                    subtitle: 'Images are accessed only after an explicit picker or camera action.',
                  ),
                  const Divider(height: 18),
                  _SettingsRow(
                    icon: Icons.admin_panel_settings_outlined,
                    title: 'System permissions',
                    subtitle: 'Review camera, microphone, photos and notification access in device settings.',
                    onTap: _permissions.openSettings,
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
                    subtitle: 'Private local-first AI for Android and iOS.',
                  ),
                  Divider(height: 18),
                  _SettingsRow(
                    icon: Icons.lock_outline_rounded,
                    title: 'Privacy model',
                    subtitle: 'Core inference, chat history, voice transcription and Canvas generation run locally. Search/Research use the network only when selected.',
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

class _LocalUsage {
  const _LocalUsage({
    required this.conversations,
    required this.databaseBytes,
    required this.attachmentBytes,
  });

  final int conversations;
  final int databaseBytes;
  final int attachmentBytes;
}

String _formatBytes(int bytes) {
  if (bytes < 1024) return '$bytes B';
  final kb = bytes / 1024;
  if (kb < 1024) return '${kb.toStringAsFixed(1)} KB';
  final mb = kb / 1024;
  if (mb < 1024) return '${mb.toStringAsFixed(1)} MB';
  return '${(mb / 1024).toStringAsFixed(2)} GB';
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
    this.destructive = false,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final VoidCallback? onTap;
  final bool destructive;

  @override
  Widget build(BuildContext context) {
    final color = destructive ? Theme.of(context).colorScheme.error : null;
    return ListTile(
      contentPadding: EdgeInsets.zero,
      leading: OpenMindFeatureIcon(icon),
      title: Text(
        title,
        style: TextStyle(fontWeight: FontWeight.w700, color: color),
      ),
      subtitle: Padding(
        padding: const EdgeInsets.only(top: 3),
        child: Text(subtitle),
      ),
      trailing: onTap == null ? null : const Icon(Icons.chevron_right_rounded),
      onTap: onTap,
    );
  }
}
