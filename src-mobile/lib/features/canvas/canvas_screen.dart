import 'package:flutter/material.dart';

import '../../core/theme/app_theme.dart';
import '../../core/theme/openmind_ui.dart';

class CanvasScreen extends StatefulWidget {
  const CanvasScreen({super.key});

  @override
  State<CanvasScreen> createState() => _CanvasScreenState();
}

class _CanvasScreenState extends State<CanvasScreen> {
  final _prompt = TextEditingController();
  String _style = 'Natural';
  String _aspect = '1:1';

  @override
  void dispose() {
    _prompt.dispose();
    super.dispose();
  }

  void _generate() {
    FocusScope.of(context).unfocus();
    final text = _prompt.text.trim();
    if (text.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('Describe the image you want to create.')),
      );
      return;
    }
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(
        content: Text('OpenMindAI Canvas local runtime is not installed yet.'),
      ),
    );
  }

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
                        'Canvas is designed to generate images on-device with a local model. No cloud image API is used by this screen.',
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
              onStyle: (value) => setState(() => _style = value),
              onAspect: (value) => setState(() => _aspect = value),
              onGenerate: _generate,
            );
            final preview = _PreviewPanel(
              prompt: _prompt.text,
              scheme: scheme,
            );
            return SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(18, 8, 18, 28),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const OpenMindPageHeader(
                    title: 'Create locally',
                    subtitle: 'Describe an image, choose a look, and keep the workflow on your device.',
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
    required this.onStyle,
    required this.onAspect,
    required this.onGenerate,
  });

  final TextEditingController prompt;
  final String style;
  final String aspect;
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
                    onSelected: (_) => onStyle(item),
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
            onSelectionChanged: (value) => onAspect(value.first),
          ),
          const SizedBox(height: 22),
          SizedBox(
            width: double.infinity,
            child: FilledButton.icon(
              onPressed: onGenerate,
              icon: const Icon(Icons.auto_awesome_rounded),
              label: const Text('Generate locally'),
            ),
          ),
          const SizedBox(height: 10),
          Text(
            'Generation runtime will be checked before work starts.',
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
  const _PreviewPanel({required this.prompt, required this.scheme});

  final String prompt;
  final ColorScheme scheme;

  @override
  Widget build(BuildContext context) {
    return OpenMindSectionCard(
      padding: const EdgeInsets.all(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          AspectRatio(
            aspectRatio: 1,
            child: Container(
              decoration: BoxDecoration(
                gradient: LinearGradient(
                  begin: Alignment.topLeft,
                  end: Alignment.bottomRight,
                  colors: [
                    AppTheme.accent.withValues(alpha: .18),
                    scheme.surfaceContainer,
                    scheme.surface,
                  ],
                ),
                borderRadius: BorderRadius.circular(18),
                border: Border.all(color: scheme.outlineVariant),
              ),
              child: const Center(
                child: Column(
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
              IconButton(onPressed: null, icon: const Icon(Icons.share_outlined)),
              IconButton(onPressed: null, icon: const Icon(Icons.download_rounded)),
            ],
          ),
        ],
      ),
    );
  }
}
