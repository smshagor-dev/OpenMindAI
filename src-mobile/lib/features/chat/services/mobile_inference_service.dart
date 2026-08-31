import 'dart:async';

import 'package:lib_llama_cpp/lib_llama_cpp.dart';

import '../../../core/constants/model_catalog.dart';
import '../../../core/services/model_storage_service.dart';
import '../models/chat_models.dart';
import 'attachment_context_service.dart';
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

abstract class MobileInferenceService {
  Stream<String> stream(MobileInferenceRequest request);
  Future<void> cancel();

  Future<String> generate(MobileInferenceRequest request) async {
    final buffer = StringBuffer();
    await for (final delta in stream(request)) {
      buffer.write(delta);
    }
    final text = buffer.toString().trim();
    if (text.isEmpty) {
      throw const MobileInferenceUnavailable('Local runtime returned no text.');
    }
    return text;
  }
}

/// Direct Android/iOS llama.cpp runtime. Model files are downloaded and
/// verified by [ModelStorageService], then mounted from app-private storage.
/// Image requests automatically route to the OpenMindAI vision model and its
/// matching mmproj file without exposing upstream model names in the UI.
class NativeMobileInferenceService extends MobileInferenceService {
  NativeMobileInferenceService({
    ModelStorageService? storage,
    AttachmentContextService? attachments,
    WebEvidenceService? webEvidence,
  })  : _storage = storage ?? ModelStorageService(),
        _attachments = attachments ?? AttachmentContextService(),
        _webEvidence = webEvidence ?? WebEvidenceService();

  final ModelStorageService _storage;
  final AttachmentContextService _attachments;
  final WebEvidenceService _webEvidence;

  StreamSubscription<LlamaResponseStreamEvent>? _activeSubscription;
  StreamController<String>? _activeController;

  @override
  Stream<String> stream(MobileInferenceRequest request) {
    final controller = StreamController<String>();
    unawaited(_start(request, controller));
    controller.onCancel = () async {
      if (identical(_activeController, controller)) {
        await cancel();
      }
    };
    return controller.stream;
  }

  Future<void> _start(
    MobileInferenceRequest request,
    StreamController<String> controller,
  ) async {
    await cancel();
    _activeController = controller;

    try {
      final prepared = await _attachments.prepare(request.attachmentPaths);
      final selected = MobileModelCatalog.byId(request.modelId);
      final runtimeModel = prepared.imagePaths.isNotEmpty ? MobileModelCatalog.vision : selected;
      final installed = await _storage.installed(runtimeModel);
      if (installed == null) {
        final suffix = prepared.imagePaths.isNotEmpty
            ? ' Install ${runtimeModel.name} to use image understanding.'
            : ' Install it from Models before chatting.';
        throw MobileInferenceUnavailable('${runtimeModel.name} is not installed.$suffix');
      }

      String webContext = '';
      if (request.mode == 'web-search' || request.mode == 'research') {
        final query = request.messages.lastWhere(
          (message) => message.role == 'user',
          orElse: () => ChatMessage(id: '', role: 'user', text: '', createdAt: DateTime.now()),
        ).text;
        if (query.trim().isNotEmpty) {
          try {
            final evidence = await _webEvidence.search(query, deep: request.mode == 'research');
            webContext = _webEvidence.formatForPrompt(evidence);
          } catch (_) {
            webContext = 'No web evidence could be retrieved for this request.';
          }
        }
      }

      final client = LlamaOpenAIClient(
        models: {
          runtimeModel.id: LlamaModelConfig(
            modelPath: installed.modelPath,
            mmprojPath: installed.mmprojPath,
          ),
        },
      );

      final input = <LlamaResponseInputItem>[
        LlamaResponseInputItem(
          role: 'system',
          content: [LlamaTextPart(_systemPrompt(request.mode, webContext: webContext))],
        ),
      ];

      for (var index = 0; index < request.messages.length; index++) {
        final message = request.messages[index];
        final isLast = index == request.messages.length - 1;
        final includeAttachments = isLast && message.role == 'user';
        final text = includeAttachments && prepared.textContext.isNotEmpty
            ? '${message.text}\n${prepared.textContext}'
            : message.text;

        if (includeAttachments && prepared.imagePaths.isNotEmpty) {
          input.add(LlamaResponseInputItem(
            role: message.role,
            content: [
              LlamaTextPart(text),
              ...prepared.imagePaths.map((path) => LlamaImageFilePart(path: path)),
            ],
          ));
        } else {
          input.add(LlamaResponseInputItem(
            role: message.role,
            content: [LlamaTextPart(text)],
          ));
        }
      }

      var emitted = false;
      final events = client.responses.stream(model: runtimeModel.id, input: input);
      final subscription = events.listen(
        (event) {
          if (!identical(_activeController, controller) || controller.isClosed) return;
          if (event is LlamaResponseOutputTextDelta) {
            emitted = true;
            controller.add(event.delta);
          }
        },
        onError: (Object error, StackTrace stackTrace) {
          if (!controller.isClosed) controller.addError(_friendlyRuntimeError(error), stackTrace);
        },
        onDone: () {
          if (!controller.isClosed) {
            if (!emitted) {
              controller.addError(const MobileInferenceUnavailable('Local runtime returned no text.'));
            }
            controller.close();
          }
          if (identical(_activeController, controller)) {
            _activeController = null;
            _activeSubscription = null;
          }
        },
        cancelOnError: true,
      );
      _activeSubscription = subscription;
    } catch (error, stackTrace) {
      if (!controller.isClosed) {
        controller.addError(_friendlyRuntimeError(error), stackTrace);
        await controller.close();
      }
      if (identical(_activeController, controller)) {
        _activeController = null;
        _activeSubscription = null;
      }
    }
  }

  @override
  Future<void> cancel() async {
    final subscription = _activeSubscription;
    _activeSubscription = null;
    if (subscription != null) await subscription.cancel();
    final controller = _activeController;
    _activeController = null;
    if (controller != null && !controller.isClosed) await controller.close();
  }

  String _systemPrompt(String mode, {required String webContext}) {
    const base = 'You are OpenMindAI, a private local-first assistant. Be accurate, concise, and useful. '
        'Never reveal internal upstream model repository names or raw model filenames to the user.';
    switch (mode) {
      case 'thinking':
        return '$base Reason carefully before answering. Give the final answer without exposing private chain-of-thought.';
      case 'web-search':
        return '$base Answer from the supplied web evidence when it is relevant. Cite sources inline as [1], [2], etc. '
            'Do not invent citations or claims not supported by the evidence.\n\n$webContext';
      case 'research':
        return '$base Produce a deeper synthesis using the supplied web evidence. Distinguish supported facts from uncertainty, '
            'cite evidence inline as [1], [2], etc., and do not fabricate sources.\n\n$webContext';
      default:
        return base;
    }
  }

  Object _friendlyRuntimeError(Object error) {
    if (error is MobileInferenceUnavailable) return error;
    final text = error.toString();
    if (text.contains('library') || text.contains('native')) {
      return const MobileInferenceUnavailable(
        'The local AI runtime could not start on this device. Check device compatibility and reinstall the app if the problem continues.',
      );
    }
    return MobileInferenceUnavailable('Local inference failed: $text');
  }
}

class MobileInferenceUnavailable implements Exception {
  const MobileInferenceUnavailable(this.message);
  final String message;

  @override
  String toString() => message;
}
