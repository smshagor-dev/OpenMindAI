class MobileModel {
  const MobileModel({
    required this.id,
    required this.name,
    required this.kind,
    required this.minRamGb,
    required this.description,
  });

  final String id;
  final String name;
  final String kind;
  final int minRamGb;
  final String description;
}

/// Public-facing names mirror the desktop catalog. Upstream repository and
/// original model names intentionally stay outside the mobile UI contract.
class MobileModelCatalog {
  static const models = <MobileModel>[
    MobileModel(
      id: 'qwen3-06b-q4',
      name: 'OpenMindAI Nano',
      kind: 'Chat',
      minRamGb: 4,
      description: 'Ultra-light assistant for low-memory phones and tablets.',
    ),
    MobileModel(
      id: 'qwen3-17b-q4km',
      name: 'OpenMindAI Swift',
      kind: 'Chat',
      minRamGb: 6,
      description: 'Fast everyday chat, writing, reasoning, and coding.',
    ),
    MobileModel(
      id: 'qwen3-4b-q4km',
      name: 'OpenMindAI Core',
      kind: 'Chat',
      minRamGb: 8,
      description: 'Balanced default model for capable modern devices.',
    ),
    MobileModel(
      id: 'qwen3-8b-q4km',
      name: 'OpenMindAI Titan',
      kind: 'Chat',
      minRamGb: 16,
      description: 'Higher-quality general model for high-memory devices.',
    ),
    MobileModel(
      id: 'deepseek-r1-15b-q4km',
      name: 'OpenMindAI Reasoning Mini',
      kind: 'Reasoning',
      minRamGb: 6,
      description: 'Lightweight dedicated reasoning for math, logic, and code.',
    ),
    MobileModel(
      id: 'deepseek-r1-7b-q4km',
      name: 'OpenMindAI Reasoning',
      kind: 'Reasoning',
      minRamGb: 16,
      description: 'Deeper local reasoning for analysis, math, and coding.',
    ),
    MobileModel(
      id: 'qwen25-vl-3b-q4km',
      name: 'OpenMindAI Lens',
      kind: 'Vision',
      minRamGb: 8,
      description: 'Local image, screenshot, chart, and document understanding.',
    ),
  ];

  static MobileModel byId(String? id) => models.firstWhere(
        (model) => model.id == id,
        orElse: () => models.first,
      );

  static MobileModel recommendForRam(int ramGb) {
    if (ramGb >= 16) return byId('qwen3-8b-q4km');
    if (ramGb >= 8) return byId('qwen3-4b-q4km');
    if (ramGb >= 6) return byId('qwen3-17b-q4km');
    return byId('qwen3-06b-q4');
  }
}
