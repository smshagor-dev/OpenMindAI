import 'dart:async';

import 'package:lib_llama_cpp/lib_llama_cpp.dart';

import '../../../core/constants/model_catalog.dart';
import '../../../core/services/model_storage_service.dart';
import '../models/chat_models.dart';
import 'attachment_context_service.dart';
import 'mounted_text_runtime.dart';
import 'web_evidence_service.dart';

class MobileInferenceRequest {
  const MobileInferenceRequest({required this.modelId, required this.mode, required this.messages, required this.attachmentPaths});
  final String modelId;
  final String mode;
  final List<ChatMessage> messages;
  final List<String> attachmentPaths;
}

abstract class MobileInferenceService {
  Stream<String> stream(MobileInferenceRequest request);
  Future<void> cancel();
  Future<void> shutdown() => cancel();
}

class MobileInferenceUnavailable implements Exception {
  const MobileInferenceUnavailable(this.message);
  final String message;
  @override
  String toString() => message;
}
