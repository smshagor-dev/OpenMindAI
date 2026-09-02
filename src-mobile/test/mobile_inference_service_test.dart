import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:openmindai_mobile/core/constants/model_catalog.dart';
import 'package:openmindai_mobile/core/services/device_profile_service.dart';
import 'package:openmindai_mobile/core/services/model_router_service.dart';
import 'package:openmindai_mobile/core/services/model_storage_service.dart';
import 'package:openmindai_mobile/features/chat/models/chat_models.dart';
import 'package:openmindai_mobile/features/chat/services/mobile_inference_service.dart';
import 'package:openmindai_mobile/features/chat/services/mounted_text_runtime.dart';

void main() {
  late Directory tempDir;
  late MobileDeviceProfile profile;

  setUp(() async {
    tempDir = await Directory.systemTemp.createTemp('openmindai-mobile-test-');
    profile = _profile(ramGb: 8);
  });

  tearDown(() async {
    if (await tempDir.exists()) {
      await tempDir.delete(recursive: true);
    }
  });

  test('selected model installed uses selected model', () async {
    final storage = _FakeModelStorage(tempDir);
    final selected = MobileModelCatalog.byId('qwen3-17b-q4km');
    await storage.add(selected);
    final runtime = _FakeRuntime({
      'qwen3-17b-q4km': ['hello'],
    });
    final service = _service(storage, runtime, profile);

    final output = await service.generate(_request(selected.id));

    expect(output, 'hello');
    expect(runtime.modelAttempts, ['qwen3-17b-q4km']);
  });

  test(
    'selected model missing falls back to installed compatible model',
    () async {
      final storage = _FakeModelStorage(tempDir);
      await storage.add(MobileModelCatalog.byId('qwen3-06b-q4'));
      final runtime = _FakeRuntime({
        'qwen3-06b-q4': ['fallback'],
      });
      final service = _service(storage, runtime, profile);

      final output = await service.generate(_request('qwen3-4b-q4km'));

      expect(output, 'fallback');
      expect(runtime.modelAttempts, ['qwen3-06b-q4']);
    },
  );

  test(
    'incompatible selected model falls back to compatible installed model',
    () async {
      profile = _profile(ramGb: 4);
      final storage = _FakeModelStorage(tempDir);
      await storage.add(MobileModelCatalog.byId('qwen3-4b-q4km'));
      await storage.add(MobileModelCatalog.byId('qwen3-06b-q4'));
      final runtime = _FakeRuntime({
        'qwen3-06b-q4': ['small'],
      });
      final service = _service(storage, runtime, profile);

      final output = await service.generate(_request('qwen3-4b-q4km'));

      expect(output, 'small');
      expect(runtime.modelAttempts, ['qwen3-06b-q4']);
    },
  );

  test('no model installed returns model not installed error', () async {
    final storage = _FakeModelStorage(tempDir);
    final runtime = _FakeRuntime(const {});
    final service = _service(storage, runtime, profile);

    expect(
      service.generate(_request('qwen3-06b-q4')),
      throwsA(
        isA<MobileInferenceException>().having(
          (error) => error.code,
          'code',
          MobileInferenceErrorCode.modelNotInstalled,
        ),
      ),
    );
  });

  test('runtime start failure retries once with fallback model', () async {
    final storage = _FakeModelStorage(tempDir);
    await storage.add(MobileModelCatalog.byId('qwen3-17b-q4km'));
    await storage.add(MobileModelCatalog.byId('qwen3-06b-q4'));
    final runtime = _FakeRuntime({
      'qwen3-17b-q4km': StateError('server failed to start'),
      'qwen3-06b-q4': ['recovered'],
    });
    final service = _service(storage, runtime, profile);

    final output = await service.generate(_request('qwen3-17b-q4km'));

    expect(output, 'recovered');
    expect(runtime.modelAttempts, ['qwen3-17b-q4km', 'qwen3-06b-q4']);
    expect(runtime.unmounts, 1);
  });

  test('single installed model runtime failure is not reported as missing', () {
    final storage = _FakeModelStorage(tempDir);
    final model = MobileModelCatalog.byId('qwen3-06b-q4');
    final runtime = _FakeRuntime({
      'qwen3-06b-q4': StateError('server failed to start'),
    });
    final service = _service(storage, runtime, profile);

    expect(
      () async {
        await storage.add(model);
        await service.generate(_request(model.id));
      }(),
      throwsA(
        isA<MobileInferenceException>().having(
          (error) => error.code,
          'code',
          MobileInferenceErrorCode.runtimeStartFailed,
        ),
      ),
    );
  });
}

MobileInferenceRequest _request(String preferredModelId) {
  return MobileInferenceRequest(
    preferredModelId: preferredModelId,
    mode: 'chat',
    messages: [
      ChatMessage(
        id: 'user-1',
        role: 'user',
        text: 'Hello',
        createdAt: DateTime(2026),
      ),
    ],
    attachmentPaths: const [],
  );
}

NativeMobileInferenceService _service(
  _FakeModelStorage storage,
  _FakeRuntime runtime,
  MobileDeviceProfile profile,
) {
  final device = _FakeDeviceProfileService(profile);
  return NativeMobileInferenceService(
    storage: storage,
    device: device,
    router: MobileModelRouter(storage: storage, device: device),
    runtime: runtime,
  );
}

MobileDeviceProfile _profile({required int ramGb}) {
  return MobileDeviceProfile(
    deviceName: 'Test phone',
    platform: 'test',
    osVersion: '1',
    ramMb: ramGb * 1024,
    freeDiskBytes: 64 * 1024 * 1024 * 1024,
    recommendedModel: MobileModelCatalog.recommendForRam(ramGb),
  );
}

class _FakeDeviceProfileService extends DeviceProfileService {
  _FakeDeviceProfileService(this.profile);

  final MobileDeviceProfile profile;

  @override
  Future<MobileDeviceProfile> read() async => profile;
}

class _FakeModelStorage extends ModelStorageService {
  _FakeModelStorage(this.tempDir);

  final Directory tempDir;
  final Map<String, InstalledMobileModel> _installed = {};

  Future<void> add(MobileModel model) async {
    final directory = Directory('${tempDir.path}/${model.id}');
    await directory.create(recursive: true);
    final file = File('${directory.path}/${model.id}.gguf');
    await file.writeAsString('model');
    String? mmprojPath;
    if (model.supportsVision) {
      final projector = File('${directory.path}/mmproj-${model.id}.gguf');
      await projector.writeAsString('projector');
      mmprojPath = projector.path;
    }
    _installed[model.id] = InstalledMobileModel(
      model: model,
      modelPath: file.path,
      mmprojPath: mmprojPath,
    );
  }

  @override
  Future<InstalledMobileModel?> installed(MobileModel model) async {
    return _installed[model.id];
  }
}

class _FakeRuntime extends MountedTextRuntime {
  _FakeRuntime(this.responses);

  final Map<String, Object> responses;
  final List<String> modelAttempts = [];
  int unmounts = 0;

  @override
  Stream<String> stream({
    required String modelId,
    required String modelPath,
    required List<Map<String, dynamic>> messages,
  }) async* {
    modelAttempts.add(modelId);
    final response = responses[modelId];
    if (response is Object && response is! List<String>) {
      throw response;
    }
    for (final delta in response as List<String>? ?? const <String>[]) {
      yield delta;
    }
  }

  @override
  Future<void> cancel() async {}

  @override
  Future<void> unmount() async {
    unmounts += 1;
  }
}
