use std::{fs, path::Path};

use crate::{
    app_error::AppError,
    hardware::{HardwareProfile, HardwareProfiler},
    model_catalog::{entry_by_id, installed_file_for_pattern, wildcard_match, ModelCatalogEntry},
    model_download::{
        ensure_contained, validate_installed_dependencies, QwenModelManifest, VerificationState,
    },
    portable_root::{available_bytes_for_path, PortableRootManager},
};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreflightStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
struct PreflightCheck {
    label: &'static str,
    status: PreflightStatus,
    detail: String,
}

#[derive(Debug, Clone)]
pub(crate) struct MediaPreflightReport {
    model_name: String,
    checks: Vec<PreflightCheck>,
}

impl MediaPreflightReport {
    pub(crate) fn ensure_ready(&self) -> Result<(), AppError> {
        let blockers = self
            .checks
            .iter()
            .filter(|check| check.status == PreflightStatus::Error)
            .map(|check| format!("{}: {}", check.label, check.detail))
            .collect::<Vec<_>>();
        if blockers.is_empty() {
            return Ok(());
        }
        Err(AppError::ArtifactGenerationFailed(format!(
            "{} is not ready for local generation. {}",
            self.model_name,
            blockers.join("; ")
        )))
    }

    pub(crate) fn issue_summary(&self) -> Option<String> {
        let issues = self
            .checks
            .iter()
            .filter(|check| check.status != PreflightStatus::Ok)
            .map(|check| format!("{}: {}", check.label, check.detail))
            .collect::<Vec<_>>();
        (!issues.is_empty()).then(|| issues.join("; "))
    }
}

#[derive(Debug, Clone, Copy)]
struct MediaSpec {
    model_id: &'static str,
    minimum_free_bytes: u64,
    diffusion_runtime: bool,
}

pub(crate) fn preflight(
    root: &PortableRootManager,
    artifact_kind: &str,
) -> Result<Option<MediaPreflightReport>, AppError> {
    let hardware = HardwareProfiler::detect();
    preflight_for_hardware(root, artifact_kind, &hardware)
}

pub(crate) fn preflight_for_hardware(
    root: &PortableRootManager,
    artifact_kind: &str,
    hardware: &HardwareProfile,
) -> Result<Option<MediaPreflightReport>, AppError> {
    preflight_with_environment(
        root,
        artifact_kind,
        hardware,
        available_bytes_for_path(root.root()),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

fn preflight_with_environment(
    root: &PortableRootManager,
    artifact_kind: &str,
    hardware: &HardwareProfile,
    available_bytes: Option<u64>,
    os: &str,
    arch: &str,
) -> Result<Option<MediaPreflightReport>, AppError> {
    let Some(spec) = spec_for(artifact_kind) else {
        return Ok(None);
    };
    let entry = entry_by_id(spec.model_id)?;
    let mut checks = Vec::new();

    checks.push(match root.validate_root() {
        Ok(()) => check(
            PreflightStatus::Ok,
            "Storage",
            "OpenMindAI Root is writable",
        ),
        Err(error) => check(
            PreflightStatus::Error,
            "Storage",
            format!("OpenMindAI Root is not writable: {error}"),
        ),
    });

    checks.push(match available_bytes {
        Some(bytes) if bytes < spec.minimum_free_bytes => check(
            PreflightStatus::Error,
            "Free space",
            format!(
                "need at least {} free before generation, have {}",
                human_bytes(spec.minimum_free_bytes),
                human_bytes(bytes)
            ),
        ),
        Some(bytes) => check(
            PreflightStatus::Ok,
            "Free space",
            format!("{} available", human_bytes(bytes)),
        ),
        None => check(
            PreflightStatus::Warning,
            "Free space",
            "could not determine available disk space",
        ),
    });

    checks.push(if hardware.memory.total_bytes < entry.min_ram_bytes {
        check(
            PreflightStatus::Error,
            "System memory",
            format!(
                "{} requires at least {} RAM; detected {}",
                entry.name,
                human_bytes(entry.min_ram_bytes),
                human_bytes(hardware.memory.total_bytes)
            ),
        )
    } else {
        check(
            PreflightStatus::Ok,
            "System memory",
            format!("{} RAM detected", human_bytes(hardware.memory.total_bytes)),
        )
    });

    if let Some(required_vram) = entry.min_vram_bytes {
        let max_vram = hardware
            .gpus
            .iter()
            .filter(|gpu| !gpu.is_software)
            .filter_map(|gpu| gpu.dedicated_vram_bytes)
            .max()
            .unwrap_or(0);
        checks.push(if max_vram < required_vram {
            check(
                PreflightStatus::Warning,
                "GPU memory",
                format!(
                    "{} dedicated VRAM is below the {} recommendation; CPU/offload fallback may be much slower",
                    human_bytes(max_vram),
                    human_bytes(required_vram)
                ),
            )
        } else {
            check(
                PreflightStatus::Ok,
                "GPU memory",
                format!("{} dedicated VRAM available", human_bytes(max_vram)),
            )
        });
    }

    if spec.diffusion_runtime {
        checks.push(if diffusion_platform_supported(os, arch) {
            check(
                PreflightStatus::Ok,
                "Runtime platform",
                format!("{os}/{arch} has a supported stable-diffusion.cpp runtime path"),
            )
        } else {
            check(
                PreflightStatus::Error,
                "Runtime platform",
                format!("{os}/{arch} has no supported stable-diffusion.cpp prebuilt runtime"),
            )
        });
    }

    validate_model_package(root, &entry, &mut checks)?;

    Ok(Some(MediaPreflightReport {
        model_name: entry.name,
        checks,
    }))
}

fn validate_model_package(
    root: &PortableRootManager,
    entry: &ModelCatalogEntry,
    checks: &mut Vec<PreflightCheck>,
) -> Result<(), AppError> {
    let Some(download) = entry.download.as_ref() else {
        checks.push(check(
            PreflightStatus::Error,
            "Model package",
            "catalog entry has no downloadable package metadata",
        ));
        return Ok(());
    };
    let model_dir = root.resolve_relative(&download.destination_dir)?;
    let Some(primary_path) =
        installed_file_for_pattern(root, &download.destination_dir, &download.filename_pattern)
    else {
        checks.push(check(
            PreflightStatus::Error,
            "Model package",
            format!(
                "{} is not installed; open Settings > Models and download it first",
                entry.name
            ),
        ));
        return Ok(());
    };
    ensure_contained(root.root(), &primary_path)?;
    let primary_size = fs::metadata(&primary_path)?.len();
    if primary_size == 0 {
        checks.push(check(
            PreflightStatus::Error,
            "Model package",
            "primary model file is empty",
        ));
        return Ok(());
    }

    let manifest_path = model_dir.join("model-manifest.json");
    if manifest_path.is_file() {
        ensure_contained(root.root(), &manifest_path)?;
        match read_primary_manifest(&manifest_path) {
            Ok(manifest) => validate_primary_manifest(
                root,
                entry,
                &model_dir,
                &primary_path,
                primary_size,
                &manifest,
                checks,
            )?,
            Err(error) => checks.push(check(
                PreflightStatus::Error,
                "Model package",
                format!("primary model manifest is invalid: {error}"),
            )),
        }
    } else {
        checks.push(check(
            PreflightStatus::Warning,
            "Model package",
            "primary model manifest is missing; re-download is recommended for full integrity tracking",
        ));
    }

    if validate_installed_dependencies(root, entry, false)? {
        checks.push(check(
            PreflightStatus::Ok,
            "Dependencies",
            "required package dependencies are present and match their install manifest",
        ));
    } else {
        checks.push(check(
            PreflightStatus::Error,
            "Dependencies",
            "required package dependencies are missing or invalid; validate or re-download the model package",
        ));
    }

    Ok(())
}

fn read_primary_manifest(path: &Path) -> Result<QwenModelManifest, AppError> {
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|error| AppError::ModelInvalid(error.to_string()))
}

fn validate_primary_manifest(
    root: &PortableRootManager,
    entry: &ModelCatalogEntry,
    model_dir: &Path,
    primary_path: &Path,
    primary_size: u64,
    manifest: &QwenModelManifest,
    checks: &mut Vec<PreflightCheck>,
) -> Result<(), AppError> {
    if manifest.repo != entry.repo || manifest.quantization != entry.quantization {
        checks.push(check(
            PreflightStatus::Error,
            "Model package",
            "primary model manifest does not match the catalog entry",
        ));
        return Ok(());
    }
    if !wildcard_match(
        &entry.download.as_ref().unwrap().filename_pattern,
        &manifest.filename,
    ) {
        checks.push(check(
            PreflightStatus::Error,
            "Model package",
            "primary model manifest points to an unexpected filename",
        ));
        return Ok(());
    }

    let manifest_file = model_dir.join(&manifest.filename);
    ensure_contained(root.root(), &manifest_file)?;
    if !manifest_file.is_file() {
        checks.push(check(
            PreflightStatus::Error,
            "Model package",
            "primary model file recorded by the manifest is missing",
        ));
        return Ok(());
    }
    let same_primary = primary_path
        .strip_prefix(model_dir)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .is_some_and(|relative| relative == manifest.filename);
    if !same_primary || manifest.size_bytes == 0 || primary_size != manifest.size_bytes {
        checks.push(check(
            PreflightStatus::Error,
            "Model package",
            "primary model file size/path does not match its install manifest",
        ));
        return Ok(());
    }
    if manifest.verification == VerificationState::Failed
        || manifest
            .actual_sha256
            .as_deref()
            .is_none_or(|digest| digest.trim().is_empty())
        || manifest.sha256.as_deref().is_some_and(|expected| {
            !manifest
                .actual_sha256
                .as_deref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
        })
    {
        checks.push(check(
            PreflightStatus::Error,
            "Model package",
            "primary model integrity metadata indicates a failed or incomplete verification",
        ));
        return Ok(());
    }

    checks.push(check(
        PreflightStatus::Ok,
        "Model package",
        "primary model file matches its verified install manifest",
    ));
    Ok(())
}

fn spec_for(kind: &str) -> Option<MediaSpec> {
    match kind {
        "image" => Some(MediaSpec {
            model_id: "sdxl-base-1",
            minimum_free_bytes: GIB,
            diffusion_runtime: true,
        }),
        "video" => Some(MediaSpec {
            model_id: "wan21-t2v-13b",
            minimum_free_bytes: 4 * GIB,
            diffusion_runtime: true,
        }),
        "audio" => Some(MediaSpec {
            model_id: "kokoro-82m-onnx",
            minimum_free_bytes: 256 * MIB,
            diffusion_runtime: false,
        }),
        _ => None,
    }
}

fn diffusion_platform_supported(os: &str, arch: &str) -> bool {
    matches!(
        (os, arch),
        ("windows", "x86_64") | ("linux", "x86_64") | ("macos", "aarch64")
    )
}

fn check(
    status: PreflightStatus,
    label: &'static str,
    detail: impl Into<String>,
) -> PreflightCheck {
    PreflightCheck {
        label,
        status,
        detail: detail.into(),
    }
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.0} MiB", bytes as f64 / MIB as f64)
    } else {
        format!("{bytes} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{BackendProfile, CpuProfile, MemoryProfile};

    fn hardware(total_ram: u64) -> HardwareProfile {
        HardwareProfile {
            operating_system: "test".to_string(),
            architecture: "x86_64".to_string(),
            cpu: CpuProfile {
                name: "test".to_string(),
                physical_cores: Some(6),
                logical_threads: 12,
            },
            memory: MemoryProfile {
                total_bytes: total_ram,
                available_bytes: total_ram,
            },
            gpus: Vec::new(),
            primary_gpu: None,
            recommended_inference_gpu: None,
            backends: BackendProfile {
                cpu: true,
                cuda: false,
                vulkan: false,
                sycl: false,
                hip: false,
                metal: false,
            },
        }
    }

    #[test]
    fn ignores_non_media_artifact_kinds() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        assert!(preflight_with_environment(
            &root,
            "pdf",
            &hardware(16 * GIB),
            Some(10 * GIB),
            "windows",
            "x86_64"
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn missing_media_model_blocks_before_generation() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let report = preflight_with_environment(
            &root,
            "video",
            &hardware(16 * GIB),
            Some(10 * GIB),
            "windows",
            "x86_64",
        )
        .unwrap()
        .unwrap();
        let error = report.ensure_ready().unwrap_err().to_string();
        assert!(error.contains("OpenMindAI Motion"));
        assert!(error.contains("not installed"));
    }

    #[test]
    fn insufficient_ram_is_a_blocker() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let report = preflight_with_environment(
            &root,
            "image",
            &hardware(8 * GIB),
            Some(10 * GIB),
            "windows",
            "x86_64",
        )
        .unwrap()
        .unwrap();
        let error = report.ensure_ready().unwrap_err().to_string();
        assert!(error.contains("System memory"));
    }

    #[test]
    fn unsupported_diffusion_platform_is_a_blocker() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let report = preflight_with_environment(
            &root,
            "image",
            &hardware(32 * GIB),
            Some(10 * GIB),
            "linux",
            "aarch64",
        )
        .unwrap()
        .unwrap();
        let error = report.ensure_ready().unwrap_err().to_string();
        assert!(error.contains("Runtime platform"));
    }
}
