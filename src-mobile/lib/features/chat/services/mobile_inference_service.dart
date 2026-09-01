import 'dart:async';

import 'package:lib_llama_cpp/lib_llama_cpp.dart';

import '../../../core/constants/model_catalog.dart';
import '../../../core/services/model_storage_service.dart';
import '../models/chat_models.dart';
import 'attachment_context_service.dart';
import 'mounted_text_runtime.dart';
import 'web_evidence_service.dart';

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

// Temporary restore marker: previous source restored. Routing integration will be applied in next atomic update.
