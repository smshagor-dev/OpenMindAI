import 'dart:io';

import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';
import 'package:xml/xml.dart';

import '../chat/models/chat_models.dart';
import '../chat/services/mobile_inference_service.dart';

class CanvasArtifact {
  const CanvasArtifact({
    required this.svg,
    required this.path,
    required this.width,
    required this.height,
  });

  final String svg;
  final String path;
  final int width;
  final int height;
}

class CanvasGenerationService {
  CanvasGenerationService({required MobileInferenceService inference})
      : _inference = inference;

  final MobileInferenceService _inference;

  Future<CanvasArtifact> generate({
    required String modelId,
    required String prompt,
    required String style,
    required String aspect,
  }) async {
    final (width, height) = switch (aspect) {
      '4:3' => (1200, 900),
      '16:9' => (1280, 720),
      _ => (1024, 1024),
    };

    final instruction = '''
Create a polished vector illustration for the user's request.

User request: $prompt
Visual style: $style
Canvas: $width x $height

Return ONLY one complete SVG document. Requirements:
- root must be <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 $width $height">
- use only vector shapes, paths, gradients and text
- no scripts, event handlers, external URLs, embedded images, foreignObject, CSS imports or href references
- keep text concise and readable
- make the composition visually finished, not a wireframe
- do not wrap the SVG in Markdown fences and do not explain it
''';

    final raw = await _inference.generate(
      MobileInferenceRequest(
        modelId: modelId,
        mode: 'thinking',
        messages: [
          ChatMessage(
            id: 'canvas-${DateTime.now().microsecondsSinceEpoch}',
            role: 'user',
            text: instruction,
            createdAt: DateTime.now(),
          ),
        ],
        attachmentPaths: const [],
      ),
    );

    final svg = sanitizeGeneratedSvg(raw);
    final support = await getApplicationSupportDirectory();
    final directory = Directory(p.join(support.path, 'canvas'));
    if (!await directory.exists()) {
      await directory.create(recursive: true);
    }
    final file = File(
      p.join(directory.path, 'openmindai-${DateTime.now().millisecondsSinceEpoch}.svg'),
    );
    await file.writeAsString(svg, flush: true);

    return CanvasArtifact(
      svg: svg,
      path: file.path,
      width: width,
      height: height,
    );
  }
}

String sanitizeGeneratedSvg(String raw) {
  final start = raw.indexOf('<svg');
  final end = raw.lastIndexOf('</svg>');
  if (start < 0 || end < start) {
    throw const CanvasGenerationException(
      'The local model did not return a valid SVG image. Try a simpler prompt.',
    );
  }

  final svg = raw.substring(start, end + '</svg>'.length).trim();
  final lower = svg.toLowerCase();
  const forbiddenFragments = [
    '<script',
    '<foreignobject',
    '<iframe',
    '<object',
    '<embed',
    '<image',
    'javascript:',
    'data:text/html',
    'xlink:href',
    ' href=',
    'url(http',
    'url(https',
    '@import',
  ];
  if (forbiddenFragments.any(lower.contains) ||
      RegExp(r'\son[a-z]+\s*=', caseSensitive: false).hasMatch(svg)) {
    throw const CanvasGenerationException(
      'The generated image contained unsupported external or executable SVG content. Regenerate it.',
    );
  }

  try {
    final document = XmlDocument.parse(svg);
    final root = document.rootElement;
    if (root.name.local.toLowerCase() != 'svg') {
      throw const CanvasGenerationException('Generated content is not an SVG image.');
    }
    final viewBox = root.getAttribute('viewBox');
    if (viewBox == null || viewBox.trim().isEmpty) {
      throw const CanvasGenerationException('Generated SVG is missing its canvas size.');
    }
  } on CanvasGenerationException {
    rethrow;
  } catch (_) {
    throw const CanvasGenerationException(
      'The generated SVG could not be parsed. Regenerate the image.',
    );
  }

  return svg;
}

class CanvasGenerationException implements Exception {
  const CanvasGenerationException(this.message);

  final String message;

  @override
  String toString() => message;
}
