import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:path/path.dart' as p;

import '../models/chat_models.dart';

class ChatMessageBubble extends StatelessWidget {
  const ChatMessageBubble({
    super.key,
    required this.message,
    required this.speaking,
    required this.onSpeak,
    this.onRegenerate,
  });

  final ChatMessage message;
  final bool speaking;
  final VoidCallback onSpeak;
  final VoidCallback? onRegenerate;

  @override
  Widget build(BuildContext context) {
    final user = message.role == 'user';
    return Align(
      alignment: user ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        constraints: BoxConstraints(
          maxWidth: MediaQuery.sizeOf(context).width * (user ? .84 : .96),
        ),
        margin: const EdgeInsets.only(bottom: 18),
        padding: user
            ? const EdgeInsets.symmetric(horizontal: 16, vertical: 11)
            : EdgeInsets.zero,
        decoration: user
            ? BoxDecoration(
                color: Theme.of(context).brightness == Brightness.dark
                    ? const Color(0xFF303030)
                    : const Color(0xFFF1F1F1),
                borderRadius: BorderRadius.circular(20),
              )
            : null,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (message.attachmentPaths.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(bottom: 8),
                child: Wrap(
                  spacing: 7,
                  runSpacing: 7,
                  children: message.attachmentPaths
                      .map((path) => _AttachmentPreview(path: path))
                      .toList(),
                ),
              ),
            if (message.text.isEmpty && !user)
              const Padding(
                padding: EdgeInsets.symmetric(vertical: 4),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    SizedBox(
                      width: 17,
                      height: 17,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                    SizedBox(width: 9),
                    Text('Thinking…'),
                  ],
                ),
              )
            else if (user)
              SelectableText(
                message.text,
                style: const TextStyle(fontSize: 16, height: 1.45),
              )
            else
              MarkdownBody(
                data: message.text,
                selectable: true,
                styleSheet: MarkdownStyleSheet.fromTheme(Theme.of(context)).copyWith(
                  p: const TextStyle(fontSize: 16, height: 1.45),
                  code: TextStyle(
                    fontFamily: 'monospace',
                    fontSize: 14,
                    backgroundColor:
                        Theme.of(context).colorScheme.surfaceContainerHighest,
                  ),
                  codeblockDecoration: BoxDecoration(
                    color: Theme.of(context).colorScheme.surfaceContainerHighest,
                    borderRadius: BorderRadius.circular(12),
                  ),
                ),
              ),
            if (!user && message.text.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(top: 4),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton(
                      visualDensity: VisualDensity.compact,
                      tooltip: 'Copy',
                      onPressed: () => Clipboard.setData(
                        ClipboardData(text: message.text),
                      ),
                      icon: const Icon(Icons.copy_rounded, size: 18),
                    ),
                    IconButton(
                      visualDensity: VisualDensity.compact,
                      tooltip: speaking
                          ? 'Stop OpenMindAI Speak'
                          : 'Read aloud with OpenMindAI Speak',
                      onPressed: onSpeak,
                      icon: Icon(
                        speaking
                            ? Icons.stop_circle_outlined
                            : Icons.volume_up_outlined,
                        size: 19,
                      ),
                    ),
                    if (onRegenerate != null)
                      IconButton(
                        visualDensity: VisualDensity.compact,
                        tooltip: 'Regenerate',
                        onPressed: onRegenerate,
                        icon: const Icon(Icons.refresh_rounded, size: 19),
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

class _AttachmentPreview extends StatelessWidget {
  const _AttachmentPreview({required this.path});

  final String path;

  bool get _isImage => const {'.png', '.jpg', '.jpeg', '.webp'}.contains(
        p.extension(path).toLowerCase(),
      );

  @override
  Widget build(BuildContext context) {
    if (_isImage) {
      return ClipRRect(
        borderRadius: BorderRadius.circular(14),
        child: Image.file(
          File(path),
          width: 160,
          height: 120,
          fit: BoxFit.cover,
          errorBuilder: (context, error, stackTrace) => _fileChip(),
        ),
      );
    }
    return _fileChip();
  }

  Widget _fileChip() => Chip(
        visualDensity: VisualDensity.compact,
        avatar: const Icon(Icons.attach_file_rounded, size: 15),
        label: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 190),
          child: Text(p.basename(path), overflow: TextOverflow.ellipsis),
        ),
      );
}
