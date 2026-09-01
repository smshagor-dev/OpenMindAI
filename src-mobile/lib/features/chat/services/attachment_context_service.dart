import 'dart:convert';
import 'dart:io';

import 'package:archive/archive.dart';
import 'package:path/path.dart' as p;
import 'package:pdfrx_engine/pdfrx_engine.dart';
import 'package:xml/xml.dart';

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
  static const _maxDocxBytes = 32 * 1024 * 1024;
  static const _maxDocxCharacters = 120000;

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
      if (extension == '.docx') {
        final value = await _extractDocx(path);
        text
          ..writeln('\n--- Attached Word document: ${p.basename(path)} ---')
          ..writeln(value)
          ..writeln('--- End attached Word document ---');
        continue;
      }
      if (extension == '.doc') {
        text.writeln(
          '\nLegacy Word file ${p.basename(path)} cannot be decoded safely on-device. Save it as .docx and attach it again.',
        );
        continue;
      }
      if (_textExtensions.contains(extension)) {
        final file = File(path);
        if (!await file.exists()) {
          continue;
        }
        final length = await file.length();
        final readLength = length > _maxTextBytes ? _maxTextBytes : length;
        final bytes = await file
            .openRead(0, readLength)
            .fold<List<int>>(<int>[], (buffer, chunk) => buffer..addAll(chunk));
        final value = utf8.decode(bytes, allowMalformed: true);
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
    return AttachmentContext(imagePaths: images, textContext: text.toString());
  }

  Future<String> _extractPdf(String path) async {
    final file = File(path);
    if (!await file.exists()) {
      return 'The selected PDF is no longer available.';
    }
    try {
      final document = await PdfDocument.openFile(path);
      final output = StringBuffer();
      try {
        await document.loadPagesProgressively();
        for (final page in document.pages) {
          final pageText = await page.loadText();
          final value = pageText?.fullText.trim();
          if (value != null && value.isNotEmpty) {
            output
              ..writeln('Page ${page.pageNumber}')
              ..writeln(value)
              ..writeln();
          }
          if (output.length >= _maxPdfCharacters) {
            return '${output.toString().substring(0, _maxPdfCharacters)}\n'
                '[PDF text truncated for local context size]';
          }
        }
      } finally {
        await document.dispose();
      }
      return output.isEmpty
          ? 'No readable PDF text was found.'
          : output.toString();
    } catch (_) {
      return 'The PDF could not be parsed locally.';
    }
  }

  Future<String> _extractDocx(String path) async {
    final file = File(path);
    if (!await file.exists()) {
      return 'The selected Word document is no longer available.';
    }
    try {
      final size = await file.length();
      if (size > _maxDocxBytes) {
        return 'This Word document is too large to expand safely on-device.';
      }
      final archive = ZipDecoder().decodeBytes(await file.readAsBytes());
      ArchiveFile? documentFile;
      for (final entry in archive) {
        if (entry.name.replaceAll('\\', '/') == 'word/document.xml') {
          documentFile = entry;
          break;
        }
      }
      final bytes = documentFile?.readBytes();
      if (bytes == null || bytes.isEmpty) {
        return 'No readable Word document body was found.';
      }
      final document = XmlDocument.parse(
        utf8.decode(bytes, allowMalformed: true),
      );
      final output = StringBuffer();
      for (final paragraph
          in document.descendants.whereType<XmlElement>().where(
            (element) => element.name.local == 'p',
          )) {
        final line = StringBuffer();
        for (final element in paragraph.descendants.whereType<XmlElement>()) {
          switch (element.name.local) {
            case 't':
              line.write(element.innerText);
            case 'tab':
              line.write('\t');
            case 'br':
            case 'cr':
              line.write('\n');
          }
        }
        final value = line.toString().trimRight();
        if (value.trim().isNotEmpty) output.writeln(value);
        if (output.length >= _maxDocxCharacters) {
          return '${output.toString().substring(0, _maxDocxCharacters)}\n'
              '[Word document text truncated for local context size]';
        }
      }
      return output.isEmpty
          ? 'No readable Word document text was found.'
          : output.toString();
    } catch (_) {
      return 'The Word document could not be parsed locally.';
    }
  }

  bool isImagePath(String path) {
    return _imageExtensions.contains(p.extension(path).toLowerCase());
  }
}
