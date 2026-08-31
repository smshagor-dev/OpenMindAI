import 'package:flutter/services.dart';
import '../models/chat_models.dart';

class MobileInferenceRequest {
  const MobileInferenceRequest({
    required this.modelId,
    required this.mode,
    required this.messages,
    required this.attachmentPaths,
  });

  final String modelId;
  final String mode;
  final List<ChatMessage> messages;
  final List<String> attachmentPaths;
}

abstract class MobileInferenceService {
  Future<String> generate(MobileInferenceRequest request);
}

/// Flutter-facing contract for the Android/iOS local runtime bridge.
///
/// Native hosts implement `openmindai.mobile/inference -> generate` and return
/// final assistant text. Dart passes only the stable OpenMindAI catalog id, so
/// upstream repository/model names never need to appear in the mobile UI.
class NativeMobileInferenceService implements MobileInferenceService {
  static const _channel = MethodChannel('openmindai.mobile/inference');

  @override
  Future<String> generate(MobileInferenceRequest request) async {
    try {
      final response = await _channel.invokeMethod<String>('generate', {
        'modelId': request.modelId,
        'mode': request.mode,
        'attachments': request.attachmentPaths,
        'messages': request.messages
            .map((message) => {'role': message.role, 'content': message.text})
            .toList(),
      });
      if (response == null || response.trim().isEmpty) {
        throw PlatformException(
          code: 'EMPTY_RESPONSE',
          message: 'Local runtime returned no text.',
        );
      }
      return response.trim();
    } on MissingPluginException {
      throw const MobileInferenceUnavailable(
        'The Flutter UI is ready, but the Android/iOS local inference bridge has not been installed in this build yet.',
      );
    }
  }
}

class MobileInferenceUnavailable implements Exception {
  const MobileInferenceUnavailable(this.message);
  final String message;

  @override
  String toString() => message;
}
