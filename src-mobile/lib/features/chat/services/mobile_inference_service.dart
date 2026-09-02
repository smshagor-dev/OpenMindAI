import 'dart:async';
import 'dart:io';

import '../../../core/services/device_profile_service.dart';
import '../../../core/services/model_router_service.dart';
import '../../../core/services/model_storage_service.dart';
import '../models/chat_models.dart';
import 'attachment_context_service.dart';
import 'mounted_text_runtime.dart';
import 'web_evidence_service.dart';

class MobileInferenceRequest {
  const MobileInferenceRequest({
    this.preferredModelId,
    required this.mode,
    required this.messages,
    required this.attachmentPaths,
  });
  final String? preferredModelId;
  final String mode;
  final List<ChatMessage> messages;
  final List<String> attachmentPaths;
}

abstract class MobileInferenceService {
  Stream<String> stream(MobileInferenceRequest request);
  Future<String> generate(MobileInferenceRequest request);
  Future<void> cancel();
  Future<void> shutdown() => cancel();
}

class NativeMobileInferenceService implements MobileInferenceService {
  NativeMobileInferenceService({
    ModelStorageService? storage,
    MobileModelRouter? router,
    DeviceProfileService? device,
    MountedTextRuntime? runtime,
    AttachmentContextService? attachments,
    WebEvidenceService? webEvidence,
  }) : _storage = storage ?? ModelStorageService(),
       _device = device ?? DeviceProfileService(),
       _runtime = runtime ?? MountedTextRuntime(),
       _attachments = attachments ?? AttachmentContextService(),
       _webEvidence = webEvidence ?? WebEvidenceService(),
       _router =
           router ??
           MobileModelRouter(
             storage: storage ?? ModelStorageService(),
             device: device ?? DeviceProfileService(),
           );

  final ModelStorageService _storage;
  final MobileModelRouter _router;
  final DeviceProfileService _device;
  final MountedTextRuntime _runtime;
  final AttachmentContextService _attachments;
  final WebEvidenceService _webEvidence;

  @override
  Stream<String> stream(MobileInferenceRequest request) {
    final controller = StreamController<String>();
    unawaited(_streamResolved(request, controller));
    controller.onCancel = cancel;
    return controller.stream;
  }

  Future<void> _streamResolved(
    MobileInferenceRequest request,
    StreamController<String> controller,
  ) async {
    try {
      await for (final delta in _streamWithFallback(request)) {
        if (controller.isClosed) return;
        controller.add(delta);
      }
      if (!controller.isClosed) await controller.close();
    } catch (error, stackTrace) {
      if (!controller.isClosed) {
        controller.addError(_normalizeError(error), stackTrace);
        await controller.close();
      }
    }
  }

  Stream<String> _streamWithFallback(MobileInferenceRequest request) async* {
    final profile = await _device.read();
    final prepared = await _prepareMessages(request);
    final needsVision = prepared.needsVision;
    var excluded = <String>{};
    Object? firstRuntimeError;

    for (var attempt = 0; attempt < 2; attempt++) {
      final MobileModelSelection selection;
      try {
        selection = await _router.resolve(
          requestedModelId: attempt == 0 ? request.preferredModelId : null,
          needsVision: needsVision,
          taskType: request.mode,
          deviceProfile: profile,
          excludedModelIds: excluded,
        );
      } on MobileModelRoutingException {
        if (firstRuntimeError != null) throw firstRuntimeError;
        rethrow;
      }
      await _validateSelection(selection, profile, needsVision);

      var emitted = false;
      try {
        await for (final delta in _runtime.stream(
          modelId: selection.model.id,
          modelPath: selection.installed.modelPath,
          messages: prepared.messages,
        )) {
          emitted = true;
          yield delta;
        }
        return;
      } catch (error) {
        final normalized = _normalizeRuntimeError(error);
        if (attempt == 0 && !emitted) {
          firstRuntimeError = normalized;
          excluded = {...excluded, selection.model.id};
          await _runtime.unmount();
          continue;
        }
        throw normalized;
      }
    }

    throw firstRuntimeError ??
        const MobileInferenceException(
          MobileInferenceErrorCode.runtimeStartFailed,
          'Local inference could not start with any compatible installed model.',
        );
  }

  @override
  Future<String> generate(MobileInferenceRequest request) async {
    final buffer = StringBuffer();
    await for (final delta in stream(request)) {
      buffer.write(delta);
    }
    return buffer.toString();
  }

  Future<_PreparedInferenceMessages> _prepareMessages(
    MobileInferenceRequest request,
  ) async {
    final attachmentContext = await _attachments.prepare(
      request.attachmentPaths,
    );
    final messages = <Map<String, dynamic>>[
      {
        'role': 'system',
        'content': _systemPrompt(request.mode, attachmentContext.imagePaths),
      },
    ];

    if (request.mode == 'web-search' || request.mode == 'research') {
      final query = request.messages.reversed
          .firstWhere(
            (message) => message.role == 'user',
            orElse: () => request.messages.last,
          )
          .text;
      final evidence = await _webEvidence.search(
        query,
        deep: request.mode == 'research',
      );
      messages.add({
        'role': 'system',
        'content': _webEvidence.formatForPrompt(evidence),
      });
    }

    final textContext = attachmentContext.textContext.trim();
    if (textContext.isNotEmpty) {
      messages.add({'role': 'system', 'content': textContext});
    }

    for (final message in request.messages) {
      if (message.text.trim().isEmpty) continue;
      messages.add({'role': message.role, 'content': message.text});
    }

    return _PreparedInferenceMessages(
      messages: messages,
      needsVision: attachmentContext.imagePaths.isNotEmpty,
    );
  }

  String _systemPrompt(String mode, List<String> imagePaths) {
    final buffer = StringBuffer(
      'You are OpenMindAI running locally on this mobile device. '
      'Answer clearly, privately, and without claiming cloud access.',
    );
    switch (mode) {
      case 'thinking':
        buffer.write(
          ' Reason step by step internally, then provide a concise answer.',
        );
      case 'web-search':
        buffer.write(
          ' Use the provided web evidence and cite source numbers when relevant.',
        );
      case 'research':
        buffer.write(
          ' Synthesize the provided evidence into a structured research answer.',
        );
    }
    if (imagePaths.isNotEmpty) {
      buffer.write(
        ' The user attached images; use the local vision-capable model if the runtime provides image context.',
      );
    }
    return buffer.toString();
  }

  Future<void> _validateSelection(
    MobileModelSelection selection,
    MobileDeviceProfile profile,
    bool needsVision,
  ) async {
    if (needsVision && !selection.model.supportsVision) {
      throw MobileInferenceException(
        MobileInferenceErrorCode.modelNotCompatible,
        '${selection.model.name} cannot process image attachments.',
      );
    }
    if (profile.ramGb < selection.model.minRamGb) {
      throw MobileInferenceException(
        MobileInferenceErrorCode.outOfMemory,
        '${selection.model.name} needs at least ${selection.model.minRamGb} GB RAM on this device.',
      );
    }
    final installed = await _storage.installed(selection.model);
    if (installed == null) {
      throw MobileInferenceException(
        MobileInferenceErrorCode.modelNotInstalled,
        '${selection.model.name} is not installed or its metadata is invalid.',
      );
    }
    if (!await File(installed.modelPath).exists()) {
      throw MobileInferenceException(
        MobileInferenceErrorCode.modelFileMissing,
        'The model file for ${selection.model.name} is missing.',
      );
    }
    if (selection.model.supportsVision &&
        (installed.mmprojPath == null ||
            !await File(installed.mmprojPath!).exists())) {
      throw MobileInferenceException(
        MobileInferenceErrorCode.modelFileMissing,
        'The vision projector for ${selection.model.name} is missing.',
      );
    }
  }

  MobileInferenceException _normalizeError(Object error) {
    if (error is MobileInferenceException) return error;
    if (error is MobileModelRoutingException) {
      return MobileInferenceException(error.code, error.message);
    }
    return _normalizeRuntimeError(error);
  }

  MobileInferenceException _normalizeRuntimeError(Object error) {
    if (error is MobileInferenceException) return error;
    final text = error.toString().toLowerCase();
    if (text.contains('out of memory') ||
        text.contains('oom') ||
        text.contains('failed to allocate') ||
        text.contains('memory')) {
      return MobileInferenceException(
        MobileInferenceErrorCode.outOfMemory,
        'The local runtime ran out of memory. OpenMindAI tried another compatible model when one was available.',
      );
    }
    return MobileInferenceException(
      MobileInferenceErrorCode.runtimeStartFailed,
      'Local inference could not start. OpenMindAI tried another compatible installed model when one was available.',
    );
  }

  @override
  Future<void> cancel() => _runtime.cancel();

  @override
  Future<void> shutdown() => _runtime.unmount();
}

class _PreparedInferenceMessages {
  const _PreparedInferenceMessages({
    required this.messages,
    required this.needsVision,
  });

  final List<Map<String, dynamic>> messages;
  final bool needsVision;
}

class MobileInferenceException implements Exception {
  const MobileInferenceException(this.code, this.message);

  final MobileInferenceErrorCode code;
  final String message;

  bool get shouldOpenModels =>
      code == MobileInferenceErrorCode.modelNotInstalled ||
      code == MobileInferenceErrorCode.modelFileMissing ||
      code == MobileInferenceErrorCode.modelNotCompatible;

  @override
  String toString() => '${code.name}: $message';
}
