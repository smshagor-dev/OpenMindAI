import 'package:flutter/material.dart';

import '../../core/constants/model_catalog.dart';
import '../../core/services/model_storage_service.dart';
import '../../core/theme/openmind_ui.dart';

Future<void> showModelManagerSheet(
  BuildContext context, {
  required ModelStorageService storage,
  ValueChanged<String>? onModelReady,
}) {
  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    useSafeArea: true,
    showDragHandle: true,
    builder: (_) => FractionallySizedBox(
      heightFactor: .94,
      child: _ModelManagerSheet(
        storage: storage,
        onModelReady: onModelReady,
      ),
    ),
  );
}

class _ModelManagerSheet extends StatefulWidget {
  const _ModelManagerSheet({required this.storage, this.onModelReady});

  final ModelStorageService storage;
  final ValueChanged<String>? onModelReady;

  @override
  State<_ModelManagerSheet> createState() => _ModelManagerSheetState();
}

class _ModelManagerSheetState extends State<_ModelManagerSheet> {
  final Map<String, bool> _installed = {};
  final Map<String, ModelInstallProgress> _progress = {};
  final Set<String> _busy = {};
  String? _error;

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  Future<void> _refresh() async {
    final values = <String, bool>{};
    for (final model in MobileModelCatalog.models) {
      values[model.id] = await widget.storage.isInstalled(model);
    }
    if (!mounted) return;
    setState(() {
      _installed
        ..clear()
        ..addAll(values);
    });
  }

  Future<void> _install(MobileModel model) async {
    if (_busy.contains(model.id)) return;
    setState(() {
      _busy.add(model.id);
      _error = null;
    });
    try {
      await widget.storage.install(
        model,
        onProgress: (value) {
          if (mounted) setState(() => _progress[model.id] = value);
        },
      );
      if (!mounted) return;
      setState(() {
        _installed[model.id] = true;
        _progress.remove(model.id);
      });
      widget.onModelReady?.call(model.id);
    } catch (error) {
      if (mounted) setState(() => _error = error.toString());
    } finally {
      if (mounted) setState(() => _busy.remove(model.id));
    }
  }

  Future<void> _delete(MobileModel model) async {
    if (_busy.contains(model.id)) return;
    setState(() => _busy.add(model.id));
    try {
      await widget.storage.delete(model);
      if (mounted) setState(() => _installed[model.id] = false);
    } finally {
      if (mounted) setState(() => _busy.remove(model.id));
    }
  }

  @override
  Widget build(BuildContext context) {
    final installedCount = _installed.values.where((value) => value).length;
    return Column(
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(20, 2, 14, 14),
          child: OpenMindPageHeader(
            title: 'Local models',
            subtitle: '$installedCount installed · Download once, then run on this device.',
            trailing: IconButton(
              tooltip: 'Refresh models',
              onPressed: _refresh,
              icon: const Icon(Icons.refresh_rounded),
            ),
          ),
        ),
        if (_error != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
            child: Material(
              color: Theme.of(context).colorScheme.errorContainer,
              borderRadius: BorderRadius.circular(16),
              child: Padding(
                padding: const EdgeInsets.all(12),
                child: Row(
                  children: [
                    const Icon(Icons.error_outline_rounded),
                    const SizedBox(width: 10),
                    Expanded(child: Text(_error!)),
                    IconButton(
                      onPressed: () => setState(() => _error = null),
                      icon: const Icon(Icons.close_rounded),
                    ),
                  ],
                ),
              ),
            ),
          ),
        Expanded(
          child: ListView.builder(
            padding: const EdgeInsets.fromLTRB(16, 2, 16, 28),
            itemCount: MobileModelCatalog.models.length,
            itemBuilder: (context, index) {
              final model = MobileModelCatalog.models[index];
              final installed = _installed[model.id] ?? false;
              final busy = _busy.contains(model.id);
              final progress = _progress[model.id];
              return Padding(
                padding: const EdgeInsets.only(bottom: 12),
                child: _ModelCard(
                  model: model,
                  installed: installed,
                  busy: busy,
                  progress: progress,
                  onInstall: () => _install(model),
                  onDelete: () => _delete(model),
                  onCancel: () => widget.storage.cancelInstall(model.id),
                ),
              );
            },
          ),
        ),
      ],
    );
  }
}

class _ModelCard extends StatelessWidget {
  const _ModelCard({
    required this.model,
    required this.installed,
    required this.busy,
    required this.progress,
    required this.onInstall,
    required this.onDelete,
    required this.onCancel,
  });

  final MobileModel model;
  final bool installed;
  final bool busy;
  final ModelInstallProgress? progress;
  final VoidCallback onInstall;
  final VoidCallback onDelete;
  final VoidCallback onCancel;

  IconData get _icon => switch (model.kind) {
        'Reasoning' => Icons.psychology_outlined,
        'Vision' => Icons.visibility_outlined,
        _ => Icons.chat_bubble_outline_rounded,
      };

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final installProgress = progress;
    return OpenMindSectionCard(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              OpenMindFeatureIcon(_icon),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      model.name,
                      style: Theme.of(context).textTheme.titleMedium,
                    ),
                    const SizedBox(height: 3),
                    Text(
                      model.description,
                      style: Theme.of(context).textTheme.bodySmall?.copyWith(
                            color: scheme.onSurfaceVariant,
                          ),
                    ),
                  ],
                ),
              ),
              if (installed)
                const OpenMindStatusPill(
                  label: 'Installed',
                  icon: Icons.check_rounded,
                  active: true,
                ),
            ],
          ),
          const SizedBox(height: 14),
          Wrap(
            spacing: 7,
            runSpacing: 7,
            children: [
              OpenMindStatusPill(label: model.kind, icon: _icon),
              OpenMindStatusPill(
                label: '${model.minRamGb}+ GB RAM',
                icon: Icons.memory_rounded,
              ),
              OpenMindStatusPill(
                label: '~${model.sizeGb.toStringAsFixed(1)} GB',
                icon: Icons.storage_rounded,
              ),
              if (model.supportsVision)
                const OpenMindStatusPill(
                  label: 'Vision',
                  icon: Icons.image_outlined,
                ),
            ],
          ),
          if (installProgress != null) ...[
            const SizedBox(height: 16),
            LinearProgressIndicator(
              value: installProgress.progress > 0 ? installProgress.progress : null,
              minHeight: 6,
              borderRadius: BorderRadius.circular(99),
            ),
            const SizedBox(height: 7),
            Text(
              installProgress.stage,
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ],
          const SizedBox(height: 14),
          Row(
            children: [
              if (!installed && !busy)
                Expanded(
                  child: FilledButton.tonalIcon(
                    onPressed: onInstall,
                    icon: const Icon(Icons.download_rounded),
                    label: const Text('Install'),
                  ),
                )
              else if (busy)
                Expanded(
                  child: OutlinedButton.icon(
                    onPressed: onCancel,
                    icon: const Icon(Icons.stop_circle_outlined),
                    label: const Text('Cancel'),
                  ),
                )
              else ...[
                Expanded(
                  child: FilledButton.tonalIcon(
                    onPressed: () => Navigator.pop(context),
                    icon: const Icon(Icons.check_rounded),
                    label: const Text('Ready'),
                  ),
                ),
                const SizedBox(width: 9),
                IconButton.outlined(
                  tooltip: 'Delete model',
                  onPressed: onDelete,
                  icon: const Icon(Icons.delete_outline_rounded),
                ),
              ],
            ],
          ),
        ],
      ),
    );
  }
}
