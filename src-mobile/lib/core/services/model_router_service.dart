import '../constants/model_catalog.dart';
import 'model_storage_service.dart';

class MobileModelRouter {
  MobileModelRouter({ModelStorageService? storage})
      : _storage = storage ?? ModelStorageService();

  final ModelStorageService _storage;

  Future<MobileModelSelection> resolve({
    required String? requestedModelId,
    required bool needsVision,
  }) async {
    final candidates = <MobileModel>[];

    if (needsVision) {
      candidates.add(MobileModelCatalog.vision);
    }

    final requested = MobileModelCatalog.byId(requestedModelId);
    if (!candidates.contains(requested)) {
      candidates.add(requested);
    }

    for (final model in MobileModelCatalog.models) {
      if (!candidates.contains(model)) {
        candidates.add(model);
      }
    }

    for (final model in candidates) {
      final installed = await _storage.installed(model);
      if (installed != null) {
        return MobileModelSelection(model: model, installed: installed);
      }
    }

    throw const MobileModelRoutingException(
      'No compatible local model is installed. Open Models and install a recommended model.',
    );
  }
}

class MobileModelSelection {
  const MobileModelSelection({required this.model, required this.installed});

  final MobileModel model;
  final InstalledMobileModel installed;
}

class MobileModelRoutingException implements Exception {
  const MobileModelRoutingException(this.message);

  final String message;

  @override
  String toString() => message;
}
