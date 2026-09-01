import 'dart:convert';
import 'dart:typed_data';

import 'package:cross_file/cross_file.dart';
import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:share_plus/share_plus.dart';

import '../../core/storage/app_settings_controller.dart';
import '../../core/theme/app_theme.dart';
import '../../core/theme/openmind_ui.dart';
import '../chat/services/mobile_inference_service.dart';
import 'canvas_generation_service.dart';

class CanvasScreen extends StatefulWidget {
  const CanvasScreen({
    super.key,
    required this.inference,
    required this.modelId,
  });

  final MobileInferenceService inference;
  final String modelId;

  @override
  State<CanvasScreen> createState() => _CanvasScreenState();
}

class _CanvasScreenState extends State<CanvasScreen> {
  final _prompt = TextEditingController();
  final _settings = AppSettingsController.instance;
  late final CanvasGenerationService _generator;

  String _style = 'Natural';
  String _aspect = '1:1';
  CanvasArtifact? _artifact;
  bool _generating = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    _generator = CanvasGenerationService(inference: widget.inference);
  }

  @override
  void dispose() {
    _prompt.dispose();
    super.dispose();
  }

  void _haptic() {
    if (_settings.haptics) {
      HapticFeedback.selectionClick();
    }
  }

  Future<void> _generate() async {
    FocusScope.of(context).unfocus();
    final text = _prompt.text.trim();
    if (text.isEmpty || _generating) {
      if (text.isEmpty) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Describe the image you want to create.')),
        );
      }
      return;
    }

    _haptic();
    setState(() {
      _generating = true;
      _error = null;
    });
    try {
      final artifact = await _generator.generate(
        modelId: widget.modelId,
        prompt: text,
        style: _style,
        aspect: _aspect,
      );
      if (!mounted) return;
      setState(() => _artifact = artifact);
      if (_settings.haptics) HapticFeedback.mediumImpact();
    } catch (error) {
      if (!mounted) return;
      setState(() => _error = error.toString());
    } finally {
      if (mounted) setState(() => _generating = false);
    }
  }

  Future<void> _save() async {
    final artifact = _artifact;
    if (artifact == null) return;
    _haptic();
    try {
      final bytes = Uint8List.fromList(utf8.encode(artifact.svg));
      final result = await FilePicker.saveFile(
        dialogTitle: 'Save OpenMindAI Canvas image',
        fileName: 'openmindai-canvas-${DateTime.now().millisecondsSinceEpoch}.svg',
        bytes: bytes,
      );
      if (!mounted || result == null) return;
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('Canvas image saved.')),
      );
    } catch (error) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Could not save image: $error')),
      );
    }
  }

  Future<void> _share() async {
    final artifact = _artifact;
    if (artifact == null) return;
    _haptic();
    try {
      await SharePlus.instance.share(
        ShareParams(
          text: 'Generated locally with OpenMindAI Canvas',
          files: [XFile(artifact.path, mimeType: 'image/svg+xml')],
        ),
      );
    } catch (error) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Could not share image: $error')),
      );
    }
  }

  double get _previewAspect => switch (_aspect) {
        '4:3' => 4 / 3,
        '16:9' => 16 / 9,
        _ => 1,
      };

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Scaffold(
      appBar: AppBar(
        title: const Text('OpenMindAI Canvas'),
        actions: [
          IconButton(
            tooltip: 'Canvas information',
            onPressed: () => showModalBottomSheet<void>(
              context: context,
              showDragHandle: true,
              builder: (context) => const SafeArea(
                child: Padding(
                  padding: EdgeInsets.fromLTRB(22, 4, 22, 28),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        'Local image generation',
                        style: TextStyle(fontSize: 22, fontWeight: FontWeight.w800),
                      ),
                      SizedBox(height: 10),
                      Text(
                        'Canvas uses the installed OpenMindAI language model to synthesize a safe SVG illustration entirely on-device. Generated files stay local unless you explicitly save or share them.',
                      ),
                    ],
                  ),
                ),
              ),
            ),
            icon: const Icon(Icons.info_outline_rounded),
          ),
        ],
      ),
      body: SafeArea(
        top: false,
        child: LayoutBuilder(
          builder: (context, constraints) {
            final wide = constraints.maxWidth >= 720;
            final editor = _EditorPanel(
              prompt: _prompt,
              style: _style,
              aspect: _aspect,
              generating: _generating,
              error: _error,
              onStyle: (value) {
                _haptic();
                setState(() => _style = value);
              },
              onAspect: (value) {
                _haptic();
                setState(() => _aspect = value);
              },
              onGenerate: _generate,
            );
            final preview = _PreviewPanel(
              artifact: _artifact,
              aspectRatio: _previewAspect,
              scheme: scheme,
              generating: _generating,
              onSave: _artifact == null ? null : _save,
              onShare: _artifact == null ? null : _share,
            );
            return SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(18, 8, 18, 28),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const OpenMindPageHeader(
                    title: 'Create locally',
                    subtitle: 'Generate, preview, save and share private vector artwork with the model already installed on your phone.',
                  ),
                  const SizedBox(height: 20),
                  if (wide)
                    Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Expanded(child: editor),
                        const SizedBox(width: 18),
                        Expanded(child: preview),
                      ],
                    )
                  else ...[
                    editor,
                    const SizedBox(height: 18),
                    preview,
                  ],
                ],
              ),
            );
          },
        ),
      ),
    );
  }
}

class _EditorPanel extends StatelessWidget {
  const _EditorPanel({
    required this.prompt,
    required this.style,
    required this.aspect,
    required this.generating,
    required this.error,
    required this.onStyle,
    required this.onAspect,
    required this.onGenerate,
  });

  final TextEditingController prompt;
  final String style;
  final String aspect;
  final bool generating;
  final String? error;
  final ValueChanged<String> onStyle;
  final ValueChanged<String> onAspect;
  final VoidCallback onGenerate;

  @override
  Widget build(BuildContext context) {
    return OpenMindSectionCard(
      padding: const EdgeInsets.all(18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Row(
            children: [
              OpenMindFeatureIcon(Icons.auto_awesome_rounded),
              SizedBox(width: 12),
              Expanded(
                child: Text(
                  'Image prompt',
                  style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800),
                ),
              ),
            ],
          ),
          const SizedBox(height: 16),
          TextField(
            controller: prompt,
            minLines: 4,
            maxLines: 8,
            enabled: !generating,
            decoration: const InputDecoration(
              hintText: 'A cinematic city at night, reflected neon lights, detailed architecture…',
              alignLabelWithHint: true,
            ),
          ),
          const SizedBox(height: 18),
          Text('Style', style: Theme.of(context).textTheme.titleSmall),
          const SizedBox(height: 9),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: ['Natural', 'Cinematic', 'Illustration', 'Minimal']
                .map(
                  (item) => ChoiceChip(
                    label: Text(item),
                    selected: style == item,
                    onSelected: generating ? null : (_) => onStyle(item),
                  ),
                )
                .toList(),
          ),
          const SizedBox(height: 18),
          Text('Aspect ratio', style: Theme.of(context).textTheme.titleSmall),
          const SizedBox(height: 9),
          SegmentedButton<String>(
            showSelectedIcon: false,
            segments: const [
              ButtonSegment(value: '1:1', label: Text('1:1')),
              ButtonSegment(value: '4:3', label: Text('4:3')),
              ButtonSegment(value: '16:9', label: Text('16:9')),
            ],
            selected: {aspect},
            onSelectionChanged: generating ? null : (value) => onAspect(value.first),
          ),
          if (error != null) ...[
            const SizedBox(height: 16),
            Material(
              color: Theme.of(context).colorScheme.errorContainer,
              borderRadius: BorderRadius.circular(14),
              child: Padding(
                padding: const EdgeInsets.all(12),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Icon(Icons.error_outline_rounded),
                    const SizedBox(width: 9),
                    Expanded(child: Text(error!)),
                  ],
                ),
              ),
            ),
          ],
          const SizedBox(height: 22),
          SizedBox(
            width: double.infinity,
            child: FilledButton.icon(
              onPressed: generating ? null : onGenerate,
              icon: generating
                  ? const SizedBox(
                      width: 18,
                      height: 18,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : const Icon(Icons.auto_awesome_rounded),
              label: Text(generating ? 'Generating locally…' : 'Generate locally'),
            ),
          ),
          const SizedBox(height: 10),
          Text(
            'Uses the selected installed OpenMindAI model. No cloud image API is called.',
            textAlign: TextAlign.center,
            style: Theme.of(context).textTheme.bodySmall?.copyWith(
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
          ),
        ],
      ),
    );
  }
}

class _PreviewPanel extends StatelessWidget {
  const _PreviewPanel({
    required this.artifact,
    required this.aspectRatio,
    required this.scheme,
    required this.generating,
    required this.onSave,
    required this.onShare,
  });

  final CanvasArtifact? artifact;
  final double aspectRatio;
  final ColorScheme scheme;
  final bool generating;
  final VoidCallback? onSave;
  final VoidCallback? onShare;

  @override
  Widget build(BuildContext context) {
    return OpenMindSectionCard(
      padding: const EdgeInsets.all(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          AspectRatio(
            aspectRatio: aspectRatio,
            child: Container(
              clipBehavior: Clip.antiAlias,
              decoration: BoxDecoration(
                color: scheme.surfaceContainer,
                borderRadius: BorderRadius.circular(18),
                border: Border.all(color: scheme.outlineVariant),
              ),
              child: artifact != null
                  ? Padding(
                      padding: const EdgeInsets.all(8),
                      child: SvgPicture.string(artifact!.svg, fit: BoxFit.contain),
                    )
                  : Center(
                      child: generating
                          ? const Column(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                CircularProgressIndicator(strokeWidth: 2),
                                SizedBox(height: 14),
                                Text('Creating on this device…'),
                              ],
                            )
                          : const Column(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                OpenMindBrandMark(size: 64),
                                SizedBox(height: 16),
                                Text(
                                  'Canvas preview',
                                  style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800),
                                ),
                                SizedBox(height: 5),
                                Text('Your generated image will appear here.'),
                              ],
                            ),
                    ),
            ),
          ),
          const SizedBox(height: 14),
          Row(
            children: [
              const OpenMindStatusPill(
                label: 'Local-only',
                icon: Icons.offline_bolt_outlined,
                active: true,
              ),
              const Spacer(),
              IconButton(
                tooltip: 'Share image',
                onPressed: onShare,
                icon: const Icon(Icons.share_outlined),
              ),
              IconButton(
                tooltip: 'Save image',
                onPressed: onSave,
                icon: const Icon(Icons.download_rounded),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
