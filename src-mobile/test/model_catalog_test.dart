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

  test('device recommendation scales with RAM', () {
    expect(MobileModelCatalog.recommendForRam(4).name, 'OpenMindAI Nano');
    expect(MobileModelCatalog.recommendForRam(6).name, 'OpenMindAI Swift');
    expect(MobileModelCatalog.recommendForRam(8).name, 'OpenMindAI Core');
    expect(MobileModelCatalog.recommendForRam(16).name, 'OpenMindAI Titan');
  });
}
