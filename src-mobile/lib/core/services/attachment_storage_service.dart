import 'dart:io';

import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';

class AttachmentStorageService {
  Future<Directory> _root() async {
    final support = await getApplicationSupportDirectory();
    final directory = Directory(p.join(support.path, 'chat_attachments'));
    if (!await directory.exists()) {
      await directory.create(recursive: true);
    }
    return directory;
  }

  Future<List<String>> persistPaths(Iterable<String> paths) async {
    final root = await _root();
    final result = <String>[];
    for (final sourcePath in paths) {
      if (sourcePath.trim().isEmpty) continue;
      final source = File(sourcePath);
      if (!await source.exists()) continue;

      final normalizedRoot = p.normalize(root.path);
      final normalizedSource = p.normalize(source.path);
      if (p.isWithin(normalizedRoot, normalizedSource)) {
        result.add(normalizedSource);
        continue;
      }

      final originalName = p.basename(source.path);
      final safeName = originalName.replaceAll(RegExp(r'[^A-Za-z0-9._-]'), '_');
      final stamp = DateTime.now().microsecondsSinceEpoch;
      final destination = File(p.join(root.path, '$stamp-$safeName'));
      await source.copy(destination.path);
      result.add(destination.path);
    }
    return result;
  }

  Future<void> deletePaths(Iterable<String> paths) async {
    final root = await _root();
    final normalizedRoot = p.normalize(root.path);
    for (final path in paths.toSet()) {
      final normalized = p.normalize(path);
      if (!p.isWithin(normalizedRoot, normalized)) continue;
      final file = File(normalized);
      if (await file.exists()) {
        try {
          await file.delete();
        } catch (_) {
          // A stale or externally locked attachment should not block chat cleanup.
        }
      }
    }
  }

  Future<int> sizeBytes() async {
    final root = await _root();
    var total = 0;
    await for (final entity in root.list(recursive: true, followLinks: false)) {
      if (entity is File) {
        try {
          total += await entity.length();
        } catch (_) {
          // Ignore files that disappear during the scan.
        }
      }
    }
    return total;
  }

  Future<int> cleanupOrphans(Set<String> referencedPaths) async {
    final root = await _root();
    final keep = referencedPaths.map(p.normalize).toSet();
    var removed = 0;
    await for (final entity in root.list(followLinks: false)) {
      if (entity is! File) continue;
      if (keep.contains(p.normalize(entity.path))) continue;
      try {
        await entity.delete();
        removed += 1;
      } catch (_) {
        // Best-effort cleanup.
      }
    }
    return removed;
  }

  Future<void> clearAll() async {
    final root = await _root();
    if (await root.exists()) {
      await root.delete(recursive: true);
    }
  }
}
