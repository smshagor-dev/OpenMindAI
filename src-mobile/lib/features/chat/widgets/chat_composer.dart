import 'package:flutter/material.dart';
import 'package:path/path.dart' as p;

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
    return SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(10, 4, 10, 10),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: [
                  _ModeChip(
                    label: 'Chat',
                    value: 'chat',
                    selected: mode == 'chat',
                    onSelected: onModeChanged,
                  ),
                  _ModeChip(
                    label: 'Think',
                    value: 'thinking',
                    selected: mode == 'thinking',
                    onSelected: onModeChanged,
                  ),
                  _ModeChip(
                    label: 'Search',
                    value: 'web-search',
                    selected: mode == 'web-search',
                    onSelected: onModeChanged,
                  ),
                  _ModeChip(
                    label: 'Research',
                    value: 'research',
                    selected: mode == 'research',
                    onSelected: onModeChanged,
                  ),
                ],
              ),
            ),
            if (attachmentPaths.isNotEmpty)
              Align(
                alignment: Alignment.centerLeft,
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(
                    children: attachmentPaths
                        .map(
                          (path) => Padding(
                            padding: const EdgeInsets.only(right: 6, bottom: 6),
                            child: Chip(
                              avatar: const Icon(Icons.attach_file_rounded, size: 17),
                              label: Text(p.basename(path)),
                              deleteIcon: const Icon(Icons.close_rounded, size: 17),
                              onDeleted: () => onRemoveAttachment(path),
                            ),
                          ),
                        )
                        .toList(),
                  ),
                ),
              ),
            TextField(
              controller: controller,
              minLines: 1,
              maxLines: 6,
              textInputAction: TextInputAction.newline,
              decoration: InputDecoration(
                hintText: voiceListening
                    ? 'Listening with OpenMindAI Hear…'
                    : 'Message OpenMindAI',
                contentPadding: const EdgeInsets.symmetric(vertical: 11),
                prefixIcon: IconButton(
                  onPressed: generating ? null : onAdd,
                  icon: const Icon(Icons.add_rounded),
                ),
                suffixIconConstraints: const BoxConstraints(minWidth: 96),
                suffixIcon: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
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
                                  ? Icons.stop_circle_outlined
                                  : Icons.mic_none_rounded,
                            ),
                    ),
                    Padding(
                      padding: const EdgeInsets.fromLTRB(0, 6, 6, 6),
                      child: IconButton.filled(
                        onPressed: generating ? onStop : onSend,
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
              onSubmitted: (_) {
                if (!generating) onSend();
              },
            ),
            const SizedBox(height: 5),
            Text(
              voiceListening
                  ? 'Listening locally · OpenMindAI Hear'
                  : 'OpenMindAI can make mistakes. Check important information.',
              style: Theme.of(context).textTheme.labelSmall,
            ),
          ],
        ),
      ),
    );
  }
}

class _ModeChip extends StatelessWidget {
  const _ModeChip({
    required this.label,
    required this.value,
    required this.selected,
    required this.onSelected,
  });

  final String label;
  final String value;
  final bool selected;
  final ValueChanged<String> onSelected;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(right: 7, bottom: 7),
      child: ChoiceChip(
        label: Text(label),
        selected: selected,
        onSelected: (_) => onSelected(value),
        visualDensity: VisualDensity.compact,
      ),
    );
  }
}
