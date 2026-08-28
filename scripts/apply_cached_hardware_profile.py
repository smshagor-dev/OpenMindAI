from pathlib import Path

LIB = Path("src-tauri/src/lib.rs")
ARTIFACTS = Path("src-tauri/src/artifacts.rs")
PREFLIGHT = Path("src-tauri/src/media_preflight.rs")


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one anchor, found {count}")
    return text.replace(old, new, 1)


lib = LIB.read_text()
lib = replace_once(
    lib,
    "struct AppState {\n    root: PortableRootManager,\n",
    "struct AppState {\n    root: PortableRootManager,\n    // CPU/GPU/RAM topology is treated as session-stable. Re-probing hardware\n    // on every generation can repeatedly hit GPU drivers and add avoidable\n    // latency, so production hot paths reuse this startup snapshot.\n    hardware: HardwareProfile,\n",
    "AppState hardware field",
)

lib = replace_once(
    lib,
    "    let artifact_kind = if kind == \"voice\" {\n        \"audio\"\n    } else {\n        kind.as_str()\n    };\n    let (artifact, path) = {\n",
    "    let artifact_kind = if kind == \"voice\" {\n        \"audio\"\n    } else {\n        kind.as_str()\n    };\n    if let Some(report) = artifacts::media_preflight::preflight_for_hardware(\n        &state.root,\n        artifact_kind,\n        &state.hardware,\n    )? {\n        report.ensure_ready()?;\n    }\n    let (artifact, path) = {\n",
    "generation preflight",
)

# Every existing per-command probe in lib.rs has AppState available except the
# two explicit hardware/profile commands handled below. Cloning the small
# snapshot preserves current call signatures without re-entering driver APIs.
lib = lib.replace(
    "    let hardware = HardwareProfiler::detect();\n",
    "    let hardware = state.hardware.clone();\n",
)

lib = replace_once(
    lib,
    "#[tauri::command]\nfn detect_hardware() -> HardwareProfile {\n    HardwareProfiler::detect()\n}\n\n#[tauri::command]\nfn get_performance_profile() -> PerformanceProfile {\n    let hardware = state.hardware.clone();\n    PerformanceProfileManager::auto(&hardware)\n}\n",
    "#[tauri::command]\nfn detect_hardware(state: State<AppState>) -> HardwareProfile {\n    state.hardware.clone()\n}\n\n#[tauri::command]\nfn get_performance_profile(state: State<AppState>) -> PerformanceProfile {\n    PerformanceProfileManager::auto(&state.hardware)\n}\n",
    "hardware commands",
)

lib = replace_once(
    lib,
    "    let runtime = LlamaRuntimeManager::new(root.clone());\n    let downloads = ModelDownloadManager::new(root.clone());\n    let runtime_installer = RuntimeInstaller::new(root.clone());\n\n    tauri::Builder::default()\n",
    "    let runtime = LlamaRuntimeManager::new(root.clone());\n    let downloads = ModelDownloadManager::new(root.clone());\n    let runtime_installer = RuntimeInstaller::new(root.clone());\n    let hardware = HardwareProfiler::detect();\n\n    tauri::Builder::default()\n",
    "startup hardware snapshot",
)
lib = replace_once(
    lib,
    "        .manage(AppState {\n            root,\n            active_database_path: database_path,\n",
    "        .manage(AppState {\n            root,\n            hardware,\n            active_database_path: database_path,\n",
    "managed hardware snapshot",
)
LIB.write_text(lib)

artifacts = ARTIFACTS.read_text()
artifacts = replace_once(
    artifacts,
    "        if let Some(report) = media_preflight::preflight(self.root, kind)? {\n            report.ensure_ready()?;\n        }\n\n",
    "",
    "destination preflight removal",
)
ARTIFACTS.write_text(artifacts)

preflight = PREFLIGHT.read_text()
preflight = replace_once(
    preflight,
    "    hardware::{HardwareProfile, HardwareProfiler},\n",
    "    hardware::HardwareProfile,\n",
    "HardwareProfiler import",
)
preflight = replace_once(
    preflight,
    "pub(crate) fn preflight(\n    root: &PortableRootManager,\n    artifact_kind: &str,\n) -> Result<Option<MediaPreflightReport>, AppError> {\n    let hardware = HardwareProfiler::detect();\n    preflight_for_hardware(root, artifact_kind, &hardware)\n}\n\n",
    "",
    "standalone preflight probe",
)
PREFLIGHT.write_text(preflight)

# Guard the intended outcome: only the single startup probe remains in lib.rs.
remaining = LIB.read_text().count("HardwareProfiler::detect()")
if remaining != 1:
    raise SystemExit(f"expected one startup HardwareProfiler::detect(), found {remaining}")

print("Applied startup-cached hardware profile reuse.")
