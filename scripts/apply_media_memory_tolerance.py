from pathlib import Path

path = Path("src-tauri/src/media_preflight.rs")
text = path.read_text(encoding="utf-8")


def replace_once(old: str, new: str) -> None:
    global text
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected one anchor, found {count}: {old[:120]!r}")
    text = text.replace(old, new, 1)


replace_once(
    '''const GIB: u64 = 1024 * MIB;\n''',
    '''const GIB: u64 = 1024 * MIB;\nconst RAM_RESERVATION_TOLERANCE_PERCENT: u64 = 5;\n''',
)

replace_once(
    '''    checks.push(if hardware.memory.total_bytes < entry.min_ram_bytes {\n        check(\n            PreflightStatus::Error,\n            "System memory",\n            format!(\n                "{} requires at least {} RAM; detected {}",\n                entry.name,\n                human_bytes(entry.min_ram_bytes),\n                human_bytes(hardware.memory.total_bytes)\n            ),\n        )\n    } else {\n        check(\n            PreflightStatus::Ok,\n            "System memory",\n            format!("{} RAM detected", human_bytes(hardware.memory.total_bytes)),\n        )\n    });\n''',
    '''    checks.push(system_memory_check(&entry, hardware.memory.total_bytes));\n''',
)

replace_once(
    '''fn spec_for(kind: &str) -> Option<MediaSpec> {\n''',
    '''fn system_memory_check(entry: &ModelCatalogEntry, detected_bytes: u64) -> PreflightCheck {\n    if detected_bytes >= entry.min_ram_bytes {\n        return check(\n            PreflightStatus::Ok,\n            "System memory",\n            format!("{} RAM detected", human_bytes(detected_bytes)),\n        );\n    }\n\n    let tolerated_floor = entry\n        .min_ram_bytes\n        .saturating_mul(100 - RAM_RESERVATION_TOLERANCE_PERCENT)\n        / 100;\n    if detected_bytes >= tolerated_floor {\n        return check(\n            PreflightStatus::Warning,\n            "System memory",\n            format!(\n                "{} RAM detected, slightly below the {} catalog threshold; generation is allowed because OS-reported physical memory can exclude small hardware-reserved regions",\n                human_bytes(detected_bytes),\n                human_bytes(entry.min_ram_bytes)\n            ),\n        );\n    }\n\n    check(\n        PreflightStatus::Error,\n        "System memory",\n        format!(\n            "{} requires at least {} RAM; detected {}",\n            entry.name,\n            human_bytes(entry.min_ram_bytes),\n            human_bytes(detected_bytes)\n        ),\n    )\n}\n\nfn spec_for(kind: &str) -> Option<MediaSpec> {\n''',
)

replace_once(
    '''    #[test]\n    fn unsupported_diffusion_platform_is_a_blocker() {\n''',
    '''    #[test]\n    fn near_threshold_ram_shortfall_is_warning_not_blocker() {\n        let entry = entry_by_id("sdxl-base-1").unwrap();\n        let detected = entry.min_ram_bytes - 256 * MIB;\n        let memory_check = system_memory_check(&entry, detected);\n\n        assert_eq!(memory_check.status, PreflightStatus::Warning);\n        assert!(memory_check.detail.contains("hardware-reserved"));\n    }\n\n    #[test]\n    fn material_ram_shortfall_remains_blocking() {\n        let entry = entry_by_id("sdxl-base-1").unwrap();\n        let memory_check = system_memory_check(&entry, 8 * GIB);\n\n        assert_eq!(memory_check.status, PreflightStatus::Error);\n    }\n\n    #[test]\n    fn unsupported_diffusion_platform_is_a_blocker() {\n''',
)

path.write_text(text, encoding="utf-8")
Path("scripts/apply_media_memory_tolerance.py").unlink()
