import 'dart:io';

import 'package:path/path.dart' as p;
import 'package:pdf_struct_extractor/pdf_struct_extractor.dart';

class AttachmentContext {
  const AttachmentContext({
    required this.imagePaths,
    required this.textContext,
  });

  final List<String> imagePaths;
  final String textContext;
}

class AttachmentContextService {
  static const _imageExtensions = {'.png', '.jpg', '.jpeg', '.webp'};
  static const _textExtensions = {
    '.txt',
    '.md',
    '.markdown',
    '.json',
    '.yaml',
    '.yml',
    '.xml',
    '.csv',
    '.dart',
    '.ts',
    '.tsx',
    '.js',
    '.jsx',
    '.py',
    '.rs',
    '.go',
    '.java',
    '.kt',
    '.swift',
    '.c',
    '.h',
    '.cpp',
    '.hpp',
    '.css',
    '.html',
    '.sql',
    '.sh',
    '.ps1',
    '.toml',
    '.ini',
    '.env',
    '.log',
  };
  static const _maxTextBytes = 256 * 1024;
  static const _maxPdfCharacters = 120000;

  Future<AttachmentContext> prepare(List<String> paths) async {
    final images = <String>[];
    final text = StringBuffer();
    for (final path in paths) {
      final extension = p.extension(path).toLowerCase();
      if (_imageExtensions.contains(extension)) {
        images.add(path);
        continue;
      }
      if (extension == '.pdf') {
        final value = await _extractPdf(path);
        text
          ..writeln('\n--- Attached PDF: ${p.basename(path)} ---')
          ..writeln(value)
          ..writeln('--- End attached PDF ---');
        continue;
      }
      if (_textExtensions.contains(extension)) {
        final file = File(path);
        if (!await file.exists()) {
          continue;
        }
        final length = await file.length();
        final readLength = length > _maxTextBytes ? _maxTextBytes : length;
        final bytes = await file.openRead(0, readLength).fold<List<int>>(
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
      text.writeln(
        '\nAttached file ${p.basename(path)} could not be converted to local text context in this build.',
      );
    }
    return AttachmentContext(
      imagePaths: images,
      textContext: text.toString(),
    );
  }

  Future<String> _extractPdf(String path) async {
    final file = File(path);
    if (!await file.exists()) {
      return 'The selected PDF is no longer available.';
    }
    try {
      final result = await PdfStructuredExtractor.extractFromFile(path);
      final pages = result['pages'];
      if (pages is! List) {
        return 'No readable PDF text was found.';
      }
      final output = StringBuffer();
      for (final page in pages) {
        if (page is! Map) {
          continue;
        }
        final blocks = page['blocks'];
        if (blocks is! List) {
          continue;
        }
        for (final block in blocks) {
          if (block is! Map) {
            continue;
          }
          final type = block['type']?.toString();
          if (type == 'table') {
            final rows = block['rows'];
            if (rows is List) {
              for (final row in rows) {
                if (row is List) {
                  output.writeln(
                    row.map((cell) => cell.toString()).join(' | '),
                  );
                }
              }
            }
          } else {
            final value = block['text']?.toString().trim();
            if (value != null && value.isNotEmpty) {
              output.writeln(value);
            }
          }
          if (output.length >= _maxPdfCharacters) {
            return '${output.toString().substring(0, _maxPdfCharacters)}\n'
                '[PDF text truncated for local context size]';
          }
        }
      }
      return output.isEmpty
          ? 'No readable PDF text was found.'
          : output.toString();
    } catch (_) {
      return 'The PDF could not be parsed locally.';
    }
  }

  bool isImagePath(String path) {
    return _imageExtensions.contains(p.extension(path).toLowerCase());
  }
}
