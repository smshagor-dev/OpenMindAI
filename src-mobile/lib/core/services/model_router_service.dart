import '../constants/model_catalog.dart';
import 'device_profile_service.dart';
import 'model_storage_service.dart';

class MobileModelRouter {
  MobileModelRouter({
    ModelStorageService? storage,
    DeviceProfileService? device,
  }) : _storage = storage ?? ModelStorageService(),
       _device = device ?? DeviceProfileService();

  final ModelStorageService _storage;
  final DeviceProfileService _device;

  Future<MobileModelSelection> resolve({
    required String? requestedModelId,
    required bool needsVision,
    String taskType = 'chat',
    MobileDeviceProfile? deviceProfile,
    Set<String> excludedModelIds = const {},
  }) async {
    final profile = deviceProfile ?? await _device.read();
    final candidates = <MobileModel>[];

    void add(MobileModel model) {
      if (excludedModelIds.contains(model.id)) return;
      if (needsVision && !model.supportsVision) return;
      if (!candidates.contains(model)) candidates.add(model);
    }

    if (requestedModelId != null) {
      final requested = MobileModelCatalog.byId(requestedModelId);
      if (requested.id == requestedModelId) add(requested);
    }

    if (needsVision) add(MobileModelCatalog.vision);

    for (final model in _taskRecommendations(taskType, profile)) {
      add(model);
    }

    final remaining = MobileModelCatalog.models.toList()
      ..sort((a, b) => a.sizeBytes.compareTo(b.sizeBytes));
    for (final model in remaining) {
      add(model);
    }

    var sawInstalled = false;
    var sawRamMismatch = false;
    for (final model in candidates) {
      final installed = await _storage.installed(model);
      if (installed == null) continue;
      sawInstalled = true;
      if (profile.ramGb < model.minRamGb) {
        sawRamMismatch = true;
        continue;
      }
      return MobileModelSelection(model: model, installed: installed);
    }

    final code = sawInstalled
        ? sawRamMismatch
              ? MobileInferenceErrorCode.outOfMemory
              : MobileInferenceErrorCode.modelNotCompatible
        : MobileInferenceErrorCode.modelNotInstalled;
    throw MobileModelRoutingException(
      code,
      sawInstalled
          ? 'Installed local models are not compatible with this device or request. Install a recommended OpenMindAI model.'
          : 'No compatible local model is installed. Open Models and install a recommended model.',
    );
  }

  List<MobileModel> _taskRecommendations(
    String taskType,
    MobileDeviceProfile profile,
  ) {
    final recommendations = MobileModelCatalog.recommendationsForDevice(
      ramGb: profile.ramGb,
      freeDiskBytes: profile.freeDiskBytes,
    );
    if (taskType == 'thinking' || taskType == 'research') {
      return [
        ...recommendations.where((model) => model.kind == 'Reasoning'),
        ...recommendations.where((model) => model.kind != 'Reasoning'),
      ];
    }
    return recommendations;
  }
}

class MobileModelSelection {
  const MobileModelSelection({required this.model, required this.installed});

  final MobileModel model;
  final InstalledMobileModel installed;
}

class MobileModelRoutingException implements Exception {
  const MobileModelRoutingException(this.code, this.message);

  final MobileInferenceErrorCode code;
  final String message;

  @override
  String toString() => '${code.name}: $message';
}

enum MobileInferenceErrorCode {
  modelNotInstalled('MODEL_NOT_INSTALLED'),
  modelFileMissing('MODEL_FILE_MISSING'),
  modelNotCompatible('MODEL_NOT_COMPATIBLE'),
  runtimeStartFailed('RUNTIME_START_FAILED'),
  outOfMemory('OUT_OF_MEMORY');

  const MobileInferenceErrorCode(this.name);

  final String name;
}
