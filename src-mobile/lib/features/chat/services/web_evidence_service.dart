import 'package:dio/dio.dart';
import 'package:html/parser.dart' as html_parser;

class WebEvidenceItem {
  const WebEvidenceItem({
    required this.title,
    required this.url,
    required this.snippet,
    this.body,
  });
  final String title;
  final String url;
  final String snippet;
  final String? body;
}

class WebEvidenceService {
  WebEvidenceService({Dio? dio}) : _dio = dio ?? Dio();

  final Dio _dio;

  Future<List<WebEvidenceItem>> search(
    String query, {
    bool deep = false,
  }) async {
    final response = await _dio.get<String>(
      'https://html.duckduckgo.com/html/',
      queryParameters: {'q': query},
      options: Options(
        responseType: ResponseType.plain,
        headers: const {
          'User-Agent': 'Mozilla/5.0 OpenMindAI-Mobile/1.0',
          'Accept': 'text/html,application/xhtml+xml',
        },
        receiveTimeout: const Duration(seconds: 20),
      ),
    );
    final document = html_parser.parse(response.data ?? '');
    final rows = document.querySelectorAll('.result');
    final results = <WebEvidenceItem>[];
    for (final row in rows) {
      final anchor = row.querySelector('.result__a');
      if (anchor == null) continue;
      final title = anchor.text.trim();
      final rawUrl = anchor.attributes['href'] ?? '';
      final url = _unwrapDuckDuckGo(rawUrl);
      if (title.isEmpty || !url.startsWith('http')) continue;
      final snippet = row.querySelector('.result__snippet')?.text.trim() ?? '';
      results.add(WebEvidenceItem(title: title, url: url, snippet: snippet));
      if (results.length >= (deep ? 8 : 6)) break;
    }

    if (!deep || results.isEmpty) return results;

    final enriched = <WebEvidenceItem>[];
    for (var index = 0; index < results.length; index++) {
      final item = results[index];
      if (index >= 3) {
        enriched.add(item);
        continue;
      }
      enriched.add(
        WebEvidenceItem(
          title: item.title,
          url: item.url,
          snippet: item.snippet,
          body: await _readPage(item.url),
        ),
      );
    }
    return enriched;
  }

  String formatForPrompt(List<WebEvidenceItem> evidence) {
    if (evidence.isEmpty) return 'No web evidence was retrieved.';
    final buffer = StringBuffer('WEB EVIDENCE\n');
    for (var index = 0; index < evidence.length; index++) {
      final item = evidence[index];
      buffer
        ..writeln('[${index + 1}] ${item.title}')
        ..writeln('URL: ${item.url}')
        ..writeln(item.snippet);
      final body = item.body?.trim();
      if (body != null && body.isNotEmpty) buffer.writeln(body);
      buffer.writeln();
    }
    return buffer.toString();
  }

  Future<String?> _readPage(String url) async {
    try {
      final response = await _dio.get<String>(
        url,
        options: Options(
          responseType: ResponseType.plain,
          headers: const {'User-Agent': 'Mozilla/5.0 OpenMindAI-Mobile/1.0'},
          receiveTimeout: const Duration(seconds: 15),
          followRedirects: true,
          maxRedirects: 5,
        ),
      );
      final document = html_parser.parse(response.data ?? '');
      for (final element in document.querySelectorAll(
        'script,style,noscript,svg',
      )) {
        element.remove();
      }
      final text = (document.body?.text ?? '')
          .replaceAll(RegExp(r'\s+'), ' ')
          .trim();
      if (text.isEmpty) return null;
      return text.length > 8000 ? text.substring(0, 8000) : text;
    } catch (_) {
      return null;
    }
  }

  String _unwrapDuckDuckGo(String rawUrl) {
    final uri = Uri.tryParse(rawUrl);
    final target = uri?.queryParameters['uddg'];
    if (target != null && target.isNotEmpty) return target;
    return rawUrl.startsWith('//') ? 'https:$rawUrl' : rawUrl;
  }
}
