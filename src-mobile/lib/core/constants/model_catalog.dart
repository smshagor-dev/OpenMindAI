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
      description: 'Balanced default model for capable modern devices.',
      repository: 'Qwen/Qwen3-4B-GGUF',
      filenameContains: ['Q4_K_M', '.gguf'],
    ),
    MobileModel(
      id: 'qwen3-8b-q4km',
      name: 'OpenMindAI Titan',
      kind: 'Chat',
      minRamGb: 16,
      sizeBytes: 5200000000,
      description: 'Higher-quality general model for high-memory devices.',
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
      description: 'Deeper local reasoning for analysis, math, and coding.',
      repository: 'lmstudio-community/DeepSeek-R1-Distill-Qwen-7B-GGUF',
      filenameContains: ['Q4_K_M', '.gguf'],
    ),
    MobileModel(
      id: 'qwen25-vl-3b-q4km',
      name: 'OpenMindAI Lens',
      kind: 'Vision',
      minRamGb: 8,
      sizeBytes: 2775000000,
      description: 'Local image, screenshot, chart, and document understanding.',
      repository: 'ggml-org/Qwen2.5-VL-3B-Instruct-GGUF',
      filenameContains: ['Q4_K_M', '.gguf'],
      mmprojFilenameContains: ['mmproj-', 'Q8_0', '.gguf'],
    ),
  ];

  static MobileModel byId(String? id) => models.firstWhere(
        (model) => model.id == id,
        orElse: () => models.first,
      );

  static MobileModel get vision => byId('qwen25-vl-3b-q4km');

  static MobileModel recommendForRam(int ramGb) {
    if (ramGb >= 16) return byId('qwen3-8b-q4km');
    if (ramGb >= 8) return byId('qwen3-4b-q4km');
    if (ramGb >= 6) return byId('qwen3-17b-q4km');
    return byId('qwen3-06b-q4');
  }
}
