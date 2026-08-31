import 'package:flutter/material.dart';

import '../../core/constants/model_catalog.dart';
import '../../core/services/model_storage_service.dart';

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
      heightFactor: .92,
      child: _ModelManagerSheet(storage: storage, onModelReady: onModelReady),
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
    return Column(
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(20, 0, 20, 14),
          child: Row(
            children: [
              const Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text('Models', style: TextStyle(fontSize: 24, fontWeight: FontWeight.w700)),
                    SizedBox(height: 3),
                    Text('Download once, then run locally on this device.'),
                  ],
                ),
              ),
              IconButton(onPressed: _refresh, icon: const Icon(Icons.refresh_rounded)),
            ],
          ),
        ),
        if (_error != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 10),
            child: Material(
              color: Theme.of(context).colorScheme.errorContainer,
              borderRadius: BorderRadius.circular(14),
              child: Padding(
                padding: const EdgeInsets.all(12),
                child: Row(
                  children: [
                    const Icon(Icons.error_outline_rounded),
                    const SizedBox(width: 10),
                    Expanded(child: Text(_error!)),
                    IconButton(onPressed: () => setState(() => _error = null), icon: const Icon(Icons.close_rounded)),
                  ],
                ),
              ),
            ),
          ),
        Expanded(
          child: ListView.separated(
            padding: const EdgeInsets.fromLTRB(12, 0, 12, 24),
            itemCount: MobileModelCatalog.models.length,
            separatorBuilder: (_, __) => const Divider(height: 1),
            itemBuilder: (context, index) {
              final model = MobileModelCatalog.models[index];
              final installed = _installed[model.id] ?? false;
              final busy = _busy.contains(model.id);
              final progress = _progress[model.id];
              return Padding(
                padding: const EdgeInsets.symmetric(vertical: 8),
                child: ListTile(
                  contentPadding: const EdgeInsets.symmetric(horizontal: 8),
                  title: Row(
                    children: [
                      Expanded(child: Text(model.name, style: const TextStyle(fontWeight: FontWeight.w700))),
                      if (installed) const Icon(Icons.check_circle_rounded, size: 19),
                    ],
                  ),
                  subtitle: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      const SizedBox(height: 4),
                      Text(model.description),
                      const SizedBox(height: 7),
                      Text('${model.kind} · ${model.minRamGb}+ GB RAM · ~${model.sizeGb.toStringAsFixed(1)} GB'),
                      if (progress != null) ...[
                        const SizedBox(height: 10),
                        LinearProgressIndicator(value: progress.progress > 0 ? progress.progress : null),
                        const SizedBox(height: 5),
                        Text(progress.stage, style: Theme.of(context).textTheme.bodySmall),
                      ],
                    ],
                  ),
                  trailing: busy
                      ? IconButton(
                          tooltip: 'Cancel',
                          onPressed: () => widget.storage.cancelInstall(model.id),
                          icon: const Icon(Icons.stop_circle_outlined),
                        )
                      : installed
                          ? IconButton(
                              tooltip: 'Delete',
                              onPressed: () => _delete(model),
                              icon: const Icon(Icons.delete_outline_rounded),
                            )
                          : FilledButton.tonal(
                              onPressed: () => _install(model),
                              child: const Text('Install'),
                            ),
                ),
              );
            },
          ),
        ),
      ],
    );
  }
}
