import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:path/path.dart' as p;

import '../../../core/storage/app_settings_controller.dart';
import '../../../core/theme/app_theme.dart';
import '../../../core/theme/openmind_ui.dart';
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
    final compact = AppSettingsController.instance.compactChat;
    final scheme = Theme.of(context).colorScheme;
    final content = Container(
      constraints: BoxConstraints(
        maxWidth: MediaQuery.sizeOf(context).width * (user ? .84 : .94),
      ),
      margin: EdgeInsets.only(bottom: compact ? 10 : 16),
      padding: user
          ? const EdgeInsets.symmetric(horizontal: 16, vertical: 12)
          : EdgeInsets.zero,
      decoration: user
          ? BoxDecoration(
              color: Theme.of(context).brightness == Brightness.dark
                  ? AppTheme.accent.withValues(alpha: .18)
                  : AppTheme.accent.withValues(alpha: .10),
              border: Border.all(color: AppTheme.accent.withValues(alpha: .20)),
              borderRadius: const BorderRadius.only(
                topLeft: Radius.circular(20),
                topRight: Radius.circular(20),
                bottomLeft: Radius.circular(20),
                bottomRight: Radius.circular(6),
              ),
            )
          : null,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (message.attachmentPaths.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(bottom: 9),
              child: Wrap(
                spacing: 7,
                runSpacing: 7,
                children: message.attachmentPaths
                    .map((path) => _AttachmentPreview(path: path))
                    .toList(),
              ),
            ),
          if (message.text.isEmpty && !user)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 5),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const SizedBox(
                    width: 17,
                    height: 17,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  ),
                  const SizedBox(width: 9),
                  Text(
                    'Thinking…',
                    style: TextStyle(color: scheme.onSurfaceVariant),
                  ),
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
              styleSheet: MarkdownStyleSheet.fromTheme(Theme.of(context))
                  .copyWith(
                    p: const TextStyle(fontSize: 16, height: 1.48),
                    h1: const TextStyle(
                      fontSize: 24,
                      fontWeight: FontWeight.w800,
                      height: 1.25,
                    ),
                    h2: const TextStyle(
                      fontSize: 21,
                      fontWeight: FontWeight.w800,
                      height: 1.3,
                    ),
                    h3: const TextStyle(
                      fontSize: 18,
                      fontWeight: FontWeight.w800,
                      height: 1.35,
                    ),
                    code: TextStyle(
                      fontFamily: 'monospace',
                      fontSize: 14,
                      color: scheme.onSurface,
                      backgroundColor: scheme.surfaceContainer,
                    ),
                    codeblockPadding: const EdgeInsets.all(14),
                    codeblockDecoration: BoxDecoration(
                      color: scheme.surfaceContainer,
                      border: Border.all(color: scheme.outlineVariant),
                      borderRadius: BorderRadius.circular(14),
                    ),
                    blockquoteDecoration: BoxDecoration(
                      color: AppTheme.accent.withValues(alpha: .08),
                      border: const Border(
                        left: BorderSide(color: AppTheme.accent, width: 3),
                      ),
                      borderRadius: BorderRadius.circular(8),
                    ),
                  ),
            ),
          if (!user && message.text.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(top: 7),
              child: Wrap(
                spacing: 1,
                children: [
                  _ActionButton(
                    tooltip: 'Copy',
                    icon: Icons.copy_rounded,
                    onPressed: () =>
                        Clipboard.setData(ClipboardData(text: message.text)),
                  ),
                  _ActionButton(
                    tooltip: speaking
                        ? 'Stop OpenMindAI Speak'
                        : 'Read aloud with OpenMindAI Speak',
                    icon: speaking
                        ? Icons.stop_circle_outlined
                        : Icons.volume_up_outlined,
                    active: speaking,
                    onPressed: onSpeak,
                  ),
                  if (onRegenerate != null)
                    _ActionButton(
                      tooltip: 'Regenerate',
                      icon: Icons.refresh_rounded,
                      onPressed: onRegenerate!,
                    ),
                ],
              ),
            ),
        ],
      ),
    );

    return Align(
      alignment: user ? Alignment.centerRight : Alignment.centerLeft,
      child: user
          ? content
          : Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Padding(
                  padding: EdgeInsets.only(top: 2, right: 10),
                  child: OpenMindBrandMark(size: 30, compact: true),
                ),
                Flexible(child: content),
              ],
            ),
    );
  }
}

class _ActionButton extends StatelessWidget {
  const _ActionButton({
    required this.tooltip,
    required this.icon,
    required this.onPressed,
    this.active = false,
  });

  final String tooltip;
  final IconData icon;
  final VoidCallback onPressed;
  final bool active;

  @override
  Widget build(BuildContext context) {
    return IconButton(
      visualDensity: VisualDensity.compact,
      tooltip: tooltip,
      onPressed: onPressed,
      style: IconButton.styleFrom(
        backgroundColor: active
            ? AppTheme.accent.withValues(alpha: .12)
            : Colors.transparent,
        foregroundColor: active ? AppTheme.accent : null,
      ),
      icon: Icon(icon, size: 18),
    );
  }
}

class _AttachmentPreview extends StatelessWidget {
  const _AttachmentPreview({required this.path});

  final String path;

  bool get _isImage => const {
    '.png',
    '.jpg',
    '.jpeg',
    '.webp',
  }.contains(p.extension(path).toLowerCase());

  @override
  Widget build(BuildContext context) {
    if (_isImage) {
      return ClipRRect(
        borderRadius: BorderRadius.circular(16),
        child: Image.file(
          File(path),
          width: 176,
          height: 132,
          fit: BoxFit.cover,
          errorBuilder: (context, error, stackTrace) => _fileChip(context),
        ),
      );
    }
    return _fileChip(context);
  }

  Widget _fileChip(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 11, vertical: 9),
    decoration: BoxDecoration(
      color: Theme.of(context).colorScheme.surfaceContainer,
      border: Border.all(color: Theme.of(context).colorScheme.outlineVariant),
      borderRadius: BorderRadius.circular(13),
    ),
    child: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        const Icon(Icons.insert_drive_file_outlined, size: 17),
        const SizedBox(width: 7),
        ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 190),
          child: Text(p.basename(path), overflow: TextOverflow.ellipsis),
        ),
      ],
    ),
  );
}
