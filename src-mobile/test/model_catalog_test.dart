import 'package:flutter_test/flutter_test.dart';
import 'package:openmindai_mobile/core/constants/model_catalog.dart';

void main() {
  test('mobile model catalog exposes only OpenMindAI public names', () {
    for (final model in MobileModelCatalog.models) {
      expect(model.name.startsWith('OpenMindAI '), isTrue);
      expect(model.name.toLowerCase(), isNot(contains('qwen')));
      expect(model.name.toLowerCase(), isNot(contains('deepseek')));
    }
  });

  test('first-run install stays on the storage-saving Nano model', () {
    const gib = 1024 * 1024 * 1024;
    expect(
      MobileModelCatalog.initialInstallModel(
        ramGb: 16,
        freeDiskBytes: 64 * gib,
      ).name,
      'OpenMindAI Nano',
    );
  });

  test('device capability recommendation scales with RAM', () {
    expect(MobileModelCatalog.recommendForRam(4).name, 'OpenMindAI Nano');
    expect(MobileModelCatalog.recommendForRam(6).name, 'OpenMindAI Swift');
    expect(MobileModelCatalog.recommendForRam(8).name, 'OpenMindAI Core');
    expect(MobileModelCatalog.recommendForRam(16).name, 'OpenMindAI Titan');
  });

  test('device capability recommendation downgrades when storage is tight', () {
    const gib = 1024 * 1024 * 1024;
    expect(
      MobileModelCatalog.recommendForDevice(
        ramGb: 16,
        freeDiskBytes: 12 * gib,
      ).name,
      'OpenMindAI Titan',
    );
    expect(
      MobileModelCatalog.recommendForDevice(
        ramGb: 16,
        freeDiskBytes: 4 * gib,
      ).name,
      'OpenMindAI Core',
    );
    expect(
      MobileModelCatalog.recommendForDevice(
        ramGb: 8,
        freeDiskBytes: 2 * gib,
      ).name,
      'OpenMindAI Nano',
    );
  });
}
