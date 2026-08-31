import 'dart:io';
import 'package:path/path.dart' as p;

class AttachmentContext {
  const AttachmentContext({required this.imagePaths, required this.textContext});
  final List<String> imagePaths;
  final String textContext;
}

class AttachmentContextService {
  static const _imageExtensions = {'.png', '.jpg', '.jpeg', '.webp'};
  static const _textExtensions = {
    '.txt', '.md', '.markdown', '.json', '.yaml', '.yml', '.xml', '.csv',
    '.dart', '.ts', '.tsx', '.js', '.jsx', '.py', '.rs', '.go', '.java',
    '.kt', '.swift', '.c', '.h', '.cpp', '.hpp', '.css', '.html', '.sql',
    '.sh', '.ps1', '.toml', '.ini', '.env', '.log',
  };

  Future<AttachmentContext> prepare(List<String> paths) async {
    final images = <String>[];
    final text = StringBuffer();
    for (final path in paths) {
      final extension = p.extension(path).toLowerCase();
      if (_imageExtensions.contains(extension)) {
        images.add(path);
        continue;
      }
      if (_textExtensions.contains(extension)) {
        final file = File(path);
        if (!await file.exists()) continue;
        final length = await file.length();
        final bytes = await file.openRead(0, length.clamp(0, 256 * 1024)).fold<List<int>>(
          <int>[],
          (buffer, chunk) => buffer..addAll(chunk),
        );
        final value = String.fromCharCodes(bytes);
        text
          ..writeln('\n--- Attached file: ${p.basename(path)} ---')
          ..writeln(value)
          ..writeln('--- End attached file ---');
        continue;
      }
      text.writeln('\nAttached file ${p.basename(path)} could not be converted to local text context in this build.');
    }
    return AttachmentContext(imagePaths: images, textContext: text.toString());
  }

  bool isImagePath(String path) => _imageExtensions.contains(p.extension(path).toLowerCase());
}
