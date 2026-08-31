import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:lib_llama_cpp/lib_llama_cpp.dart';

class MountedTextRuntime {
  LlamaHttpServer? _server;
  Uri? _baseUri;
  String? _mountedModelId;
  String? _mountedModelPath;
  HttpClientRequest? _activeRequest;

  String? get mountedModelId => _mountedModelId;
  bool get isMounted => _server != null && _baseUri != null;

  Stream<String> stream({
    required String modelId,
    required String modelPath,
    required List<Map<String, dynamic>> messages,
  }) {
    final controller = StreamController<String>();
    unawaited(_run(
      controller,
      modelId: modelId,
      modelPath: modelPath,
      messages: messages,
    ));
    controller.onCancel = cancel;
    return controller.stream;
  }

  Future<void> _run(
    StreamController<String> controller, {
    required String modelId,
    required String modelPath,
    required List<Map<String, dynamic>> messages,
  }) async {
    try {
      final baseUri = await _ensureMounted(modelId: modelId, modelPath: modelPath);
      final client = HttpClient()..connectionTimeout = const Duration(seconds: 20);
      try {
        final uri = baseUri.resolve('chat/completions');
        final request = await client.postUrl(uri);
        _activeRequest = request;
        request.headers.contentType = ContentType.json;
        request.headers.set(HttpHeaders.acceptHeader, 'text/event-stream');
        request.write(jsonEncode({
          'model': modelId,
          'messages': messages,
          'stream': true,
        }));

        final response = await request.close();
        if (response.statusCode < 200 || response.statusCode >= 300) {
          final body = await utf8.decoder.bind(response).join();
          throw StateError('Local runtime returned ${response.statusCode}: $body');
        }

        await for (final line in response.transform(utf8.decoder).transform(const LineSplitter())) {
          if (controller.isClosed) break;
          if (!line.startsWith('data:')) continue;
          final data = line.substring(5).trim();
          if (data.isEmpty || data == '[DONE]') continue;
          final decoded = jsonDecode(data);
          if (decoded is! Map) continue;
          final choices = decoded['choices'];
          if (choices is! List || choices.isEmpty || choices.first is! Map) continue;
          final delta = (choices.first as Map)['delta'];
          if (delta is! Map) continue;
          final content = delta['content'];
          if (content is String && content.isNotEmpty) controller.add(content);
        }
      } finally {
        _activeRequest = null;
        client.close(force: true);
      }
      if (!controller.isClosed) await controller.close();
    } catch (error, stackTrace) {
      if (!controller.isClosed) {
        controller.addError(error, stackTrace);
        await controller.close();
      }
    }
  }

  Future<Uri> _ensureMounted({required String modelId, required String modelPath}) async {
    if (_server != null &&
        _baseUri != null &&
        _mountedModelId == modelId &&
        _mountedModelPath == modelPath) {
      return _baseUri!;
    }

    await unmount();
    final server = LlamaHttpServer.open(
      config: LlamaServerConfig(
        model: modelId,
        modelPath: modelPath,
        port: 0,
      ),
    );
    final address = await server.start();
    _server = server;
    _mountedModelId = modelId;
    _mountedModelPath = modelPath;
    _baseUri = Uri.parse('http://${address.host}:${address.port}/v1/');
    return _baseUri!;
  }

  Future<void> cancel() async {
    final request = _activeRequest;
    _activeRequest = null;
    request?.abort(const HttpException('Generation cancelled.'));
  }

  Future<void> unmount() async {
    await cancel();
    final server = _server;
    _server = null;
    _baseUri = null;
    _mountedModelId = null;
    _mountedModelPath = null;
    if (server != null) await server.close();
  }
}
