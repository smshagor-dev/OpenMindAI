class MobileModel {
  const MobileModel({
    required this.id,
    required this.name,
    required this.kind,
    required this.minRamGb,
    required this.sizeBytes,
    required this.description,
    required this.repository,
    required this.filenameContains,
    this.mmprojFilenameContains,
  });

  final String id;
  final String name;
  final String kind;
  final int minRamGb;
  final int sizeBytes;
  final String description;

  /// Internal provisioning metadata. Never render these values in user-facing UI.
  final String repository;
  final List<String> filenameContains;
  final List<String>? mmprojFilenameContains;

  bool get supportsVision => mmprojFilenameContains != null;
  double get sizeGb => sizeBytes / 1024 / 1024 / 1024;
}

/// Public-facing names mirror the desktop catalog. Upstream repository and
/// original model names are internal provisioning details and must never be
/// rendered by mobile UI widgets.
class MobileModelCatalog {
  static const int _installReserveBytes = 1024 * 1024 * 1024;

  static const models = <MobileModel>[
    MobileModel(
      id: 'qwen3-06b-q4',
      name: 'OpenMindAI Nano',
      kind: 'Chat',
      minRamGb: 4,
      sizeBytes: 429000000,
      description: 'Ultra-light assistant for low-memory phones and tablets.',
      repository: 'ggml-org/Qwen3-0.6B-GGUF',
      filenameContains: ['Q4_0', '.gguf'],
    ),
    MobileModel(
      id: 'qwen3-17b-q4km',
      name: 'OpenMindAI Swift',
      kind: 'Chat',
      minRamGb: 6,
      sizeBytes: 1280000000,
      description: 'Fast everyday chat, writing, reasoning, and coding.',
      repository: 'ggml-org/Qwen3-1.7B-GGUF',
      filenameContains: ['Q4_K_M', '.gguf'],
    ),
    MobileModel(
      id: 'qwen3-4b-q4km',
      name: 'OpenMindAI Core',
      kind: 'Chat',
      minRamGb: 8,
      sizeBytes: 2497280256,
      description: 'Balanced optional model for capable modern devices.',
      repository: 'Qwen/Qwen3-4B-GGUF',
      filenameContains: ['Q4_K_M', '.gguf'],
    ),
    MobileModel(
      id: 'qwen3-8b-q4km',
      name: 'OpenMindAI Titan',
      kind: 'Chat',
      minRamGb: 16,
      sizeBytes: 5200000000,
      description: 'Higher-quality optional model for high-memory devices.',
      repository: 'Qwen/Qwen3-8B-GGUF',
      filenameContains: ['Q4_K_M', '.gguf'],
    ),
    MobileModel(
      id: 'deepseek-r1-15b-q4km',
      name: 'OpenMindAI Reasoning Mini',
      kind: 'Reasoning',
      minRamGb: 6,
      sizeBytes: 1120000000,
      description: 'Lightweight dedicated reasoning for math, logic, and code.',
      repository: 'lmstudio-community/DeepSeek-R1-Distill-Qwen-1.5B-GGUF',
      filenameContains: ['Q4_K_M', '.gguf'],
    ),
    MobileModel(
      id: 'deepseek-r1-7b-q4km',
      name: 'OpenMindAI Reasoning',
      kind: 'Reasoning',
      minRamGb: 16,
      sizeBytes: 4680000000,
      description:
          'Deeper optional local reasoning for analysis, math, and coding.',
      repository: 'lmstudio-community/DeepSeek-R1-Distill-Qwen-7B-GGUF',
      filenameContains: ['Q4_K_M', '.gguf'],
    ),
    MobileModel(
      id: 'qwen25-vl-3b-q4km',
      name: 'OpenMindAI Lens',
      kind: 'Vision',
      minRamGb: 8,
      sizeBytes: 2775000000,
      description:
          'Optional local image, screenshot, chart, and document understanding.',
      repository: 'ggml-org/Qwen2.5-VL-3B-Instruct-GGUF',
      filenameContains: ['Q4_K_M', '.gguf'],
      mmprojFilenameContains: ['mmproj-', 'Q8_0', '.gguf'],
    ),
  ];

  static MobileModel byId(String? id) =>
      models.firstWhere((model) => model.id == id, orElse: () => models.first);

  static MobileModel get vision => byId('qwen25-vl-3b-q4km');

  /// Small first-run download. More capable models stay opt-in in Model Manager.
  /// This keeps a fresh mobile install near the Nano footprint instead of
  /// automatically pulling multi-gigabyte models on high-memory phones.
  static MobileModel initialInstallModel({
    required int ramGb,
    required int freeDiskBytes,
  }) {
    final nano = byId('qwen3-06b-q4');
    final requiredBytes = nano.sizeBytes + _installReserveBytes;
    if (freeDiskBytes > 0 && freeDiskBytes < requiredBytes) return nano;
    return nano;
  }

  static MobileModel recommendForRam(int ramGb) {
    if (ramGb >= 16) return byId('qwen3-8b-q4km');
    if (ramGb >= 8) return byId('qwen3-4b-q4km');
    if (ramGb >= 6) return byId('qwen3-17b-q4km');
    return byId('qwen3-06b-q4');
  }

  /// Capability recommendation used by the model manager. This does not mean
  /// the model should be downloaded automatically during onboarding.
  static MobileModel recommendForDevice({
    required int ramGb,
    required int freeDiskBytes,
  }) {
    return recommendationsForDevice(
      ramGb: ramGb,
      freeDiskBytes: freeDiskBytes,
    ).first;
  }

  static List<MobileModel> recommendationsForDevice({
    required int ramGb,
    required int freeDiskBytes,
  }) {
    final candidates = <MobileModel>[
      byId('qwen3-8b-q4km'),
      byId('qwen3-4b-q4km'),
      byId('qwen3-17b-q4km'),
      byId('qwen3-06b-q4'),
      byId('deepseek-r1-7b-q4km'),
      byId('deepseek-r1-15b-q4km'),
      byId('qwen25-vl-3b-q4km'),
    ];

    final recommended = <MobileModel>[];
    for (final model in candidates) {
      if (ramGb < model.minRamGb) continue;
      final requiredBytes = model.sizeBytes + _installReserveBytes;
      if (freeDiskBytes <= 0 || freeDiskBytes >= requiredBytes) {
        recommended.add(model);
      }
    }
    return recommended.isEmpty ? [byId('qwen3-06b-q4')] : recommended;
  }
}
