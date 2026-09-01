import 'package:flutter/material.dart';
import 'package:path/path.dart' as p;

import '../../../core/theme/app_theme.dart';

class ChatComposer extends StatelessWidget {
  const ChatComposer({
    super.key,
    required this.controller,
    required this.mode,
    required this.attachmentPaths,
    required this.generating,
    required this.voiceListening,
    required this.voicePreparing,
    required this.onModeChanged,
    required this.onAdd,
    required this.onVoice,
    required this.onSend,
    required this.onStop,
    required this.onRemoveAttachment,
  });

  final TextEditingController controller;
  final String mode;
  final List<String> attachmentPaths;
  final bool generating;
  final bool voiceListening;
  final bool voicePreparing;
  final ValueChanged<String> onModeChanged;
  final VoidCallback onAdd;
  final VoidCallback onVoice;
  final VoidCallback onSend;
  final VoidCallback onStop;
  final ValueChanged<String> onRemoveAttachment;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return SafeArea(
      top: false,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: Theme.of(context).scaffoldBackgroundColor,
          border: Border(
            top: BorderSide(
              color: scheme.outlineVariant.withValues(alpha: .45),
            ),
          ),
        ),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(12, 9, 12, 10),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: Row(
                  children: [
                    _ModeChip(
                      icon: Icons.chat_bubble_outline_rounded,
                      label: 'Chat',
                      value: 'chat',
                      selected: mode == 'chat',
                      onSelected: onModeChanged,
                    ),
                    _ModeChip(
                      icon: Icons.psychology_outlined,
                      label: 'Think',
                      value: 'thinking',
                      selected: mode == 'thinking',
                      onSelected: onModeChanged,
                    ),
                    _ModeChip(
                      icon: Icons.travel_explore_rounded,
                      label: 'Search',
                      value: 'web-search',
                      selected: mode == 'web-search',
                      onSelected: onModeChanged,
                    ),
                    _ModeChip(
                      icon: Icons.biotech_outlined,
                      label: 'Research',
                      value: 'research',
                      selected: mode == 'research',
                      onSelected: onModeChanged,
                    ),
                  ],
                ),
              ),
              if (attachmentPaths.isNotEmpty) ...[
                const SizedBox(height: 2),
                Align(
                  alignment: Alignment.centerLeft,
                  child: SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    child: Row(
                      children: attachmentPaths
                          .map(
                            (path) => Padding(
                              padding: const EdgeInsets.only(
                                right: 7,
                                bottom: 7,
                              ),
                              child: InputChip(
                                avatar: const Icon(
                                  Icons.attach_file_rounded,
                                  size: 16,
                                ),
                                label: ConstrainedBox(
                                  constraints: const BoxConstraints(
                                    maxWidth: 150,
                                  ),
                                  child: Text(
                                    p.basename(path),
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                  ),
                                ),
                                deleteIcon: const Icon(
                                  Icons.close_rounded,
                                  size: 16,
                                ),
                                onDeleted: () => onRemoveAttachment(path),
                              ),
                            ),
                          )
                          .toList(),
                    ),
                  ),
                ),
              ],
              Container(
                decoration: BoxDecoration(
                  color: Theme.of(context).inputDecorationTheme.fillColor,
                  borderRadius: BorderRadius.circular(25),
                  border: Border.all(color: scheme.outlineVariant),
                  boxShadow: [
                    BoxShadow(
                      color: Colors.black.withValues(
                        alpha: Theme.of(context).brightness == Brightness.dark
                            ? .16
                            : .045,
                      ),
                      blurRadius: 18,
                      offset: const Offset(0, 6),
                    ),
                  ],
                ),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.end,
                  children: [
                    Padding(
                      padding: const EdgeInsets.only(left: 5, bottom: 5),
                      child: IconButton(
                        tooltip: 'Add photo or file',
                        onPressed: generating ? null : onAdd,
                        icon: const Icon(Icons.add_rounded),
                      ),
                    ),
                    Expanded(
                      child: TextField(
                        controller: controller,
                        minLines: 1,
                        maxLines: 6,
                        textInputAction: TextInputAction.newline,
                        decoration: InputDecoration(
                          filled: false,
                          border: InputBorder.none,
                          enabledBorder: InputBorder.none,
                          focusedBorder: InputBorder.none,
                          contentPadding: const EdgeInsets.fromLTRB(
                            6,
                            13,
                            8,
                            13,
                          ),
                          hintText: voiceListening
                              ? 'Listening with OpenMindAI Hear…'
                              : 'Message OpenMindAI',
                        ),
                        onSubmitted: (_) {
                          if (!generating) onSend();
                        },
                      ),
                    ),
                    IconButton(
                      tooltip: voiceListening
                          ? 'Stop OpenMindAI Hear'
                          : 'Voice input',
                      onPressed: generating || voicePreparing ? null : onVoice,
                      icon: voicePreparing
                          ? const SizedBox(
                              width: 19,
                              height: 19,
                              child: CircularProgressIndicator(strokeWidth: 2),
                            )
                          : Icon(
                              voiceListening
                                  ? Icons.stop_circle_rounded
                                  : Icons.mic_none_rounded,
                              color: voiceListening ? AppTheme.accent : null,
                            ),
                    ),
                    Padding(
                      padding: const EdgeInsets.fromLTRB(1, 6, 7, 6),
                      child: IconButton.filled(
                        tooltip: generating ? 'Stop generation' : 'Send',
                        onPressed: generating ? onStop : onSend,
                        style: IconButton.styleFrom(
                          backgroundColor: generating
                              ? scheme.error
                              : AppTheme.accent,
                          foregroundColor: Colors.white,
                        ),
                        icon: Icon(
                          generating
                              ? Icons.stop_rounded
                              : Icons.arrow_upward_rounded,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 6),
              Row(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Icon(
                    voiceListening
                        ? Icons.graphic_eq_rounded
                        : Icons.lock_outline_rounded,
                    size: 13,
                    color: scheme.onSurfaceVariant,
                  ),
                  const SizedBox(width: 5),
                  Flexible(
                    child: Text(
                      voiceListening
                          ? 'Listening locally · OpenMindAI Hear'
                          : 'Local-first AI · Check important information',
                      overflow: TextOverflow.ellipsis,
                      style: Theme.of(context).textTheme.labelSmall?.copyWith(
                        color: scheme.onSurfaceVariant,
                      ),
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _ModeChip extends StatelessWidget {
  const _ModeChip({
    required this.icon,
    required this.label,
    required this.value,
    required this.selected,
    required this.onSelected,
  });

  final IconData icon;
  final String label;
  final String value;
  final bool selected;
  final ValueChanged<String> onSelected;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(right: 7, bottom: 7),
      child: ChoiceChip(
        avatar: Icon(icon, size: 16),
        label: Text(label),
        selected: selected,
        onSelected: (_) => onSelected(value),
        visualDensity: VisualDensity.compact,
        side: BorderSide(
          color: selected
              ? AppTheme.accent.withValues(alpha: .5)
              : Theme.of(context).colorScheme.outlineVariant,
        ),
      ),
    );
  }
}
