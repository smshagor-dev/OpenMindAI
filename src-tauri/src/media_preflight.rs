use std::{fs, path::Path};

use crate::{
    app_error::AppError,
    hardware::HardwareProfile,
    model_catalog::{entry_by_id, installed_file_for_pattern, wildcard_match, ModelCatalogEntry},
    model_download::{
        ensure_contained, format_byte_pair, format_bytes, primary_manifest_available,
        read_primary_manifest, sha256_file, validate_installed_dependencies, QwenModelManifest,
        VerificationState,
    },
    portable_root::{available_bytes_for_path, PortableRootManager},
};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const RAM_RESERVATION_TOLERANCE_PERCENT: u64 = 5;
/// VRAM within this share of the recommendation is reported as "slightly below".
const VRAM_NEAR_RECOMMENDATION_PERCENT: u64 = 99;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreflightStatus {
    Ok,
    Warning,
    Error,
}

// A diagnostic UI refresh must never claim that it rehashed multi-GB models.
// The generation gate takes the verified path and hashes the bytes in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntegrityPolicy {
    MetadataOnly,
    VerifyContents,
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

/// Called at the generation boundary. A matching stored digest is not proof
/// that the current file bytes are intact, so verify primary and dependency files.
pub(crate) fn preflight_for_hardware(
    root: &PortableRootManager,
    artifact_kind: &str,
    hardware: &HardwareProfile,
) -> Result<Option<MediaPreflightReport>, AppError> {
    preflight_with_environment_and_policy(
        root,
        artifact_kind,
        hardware,
        available_bytes_for_path(root.root()),
        std::env::consts::OS,
        std::env::consts::ARCH,
        IntegrityPolicy::VerifyContents,
    )
}

/// Diagnostics-only: quick checks disclose that hashes were not recalculated.
/// Do not use this mode to authorize a generation job.
pub(crate) fn preflight_for_hardware_fast(
    root: &PortableRootManager,
    artifact_kind: &str,
    hardware: &HardwareProfile,
) -> Result<Option<MediaPreflightReport>, AppError> {
    preflight_with_environment_and_policy(
        root,
        artifact_kind,
        hardware,
        available_bytes_for_path(root.root()),
        std::env::consts::OS,
        std::env::consts::ARCH,
        IntegrityPolicy::MetadataOnly,
    )
}

#[cfg(test)]
fn preflight_with_environment(
    root: &PortableRootManager,
    artifact_kind: &str,
    hardware: &HardwareProfile,
    available_bytes: Option<u64>,
    os: &str,
    arch: &str,
) -> Result<Option<MediaPreflightReport>, AppError> {
    preflight_with_environment_and_policy(
        root,
        artifact_kind,
        hardware,
        available_bytes,
        os,
        arch,
        IntegrityPolicy::VerifyContents,
    )
}

fn preflight_with_environment_and_policy(
    root: &PortableRootManager,
    artifact_kind: &str,
    hardware: &HardwareProfile,
    available_bytes: Option<u64>,
    os: &str,
    arch: &str,
    integrity: IntegrityPolicy,
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
        Some(bytes) if bytes < spec.minimum_free_bytes => {
            let (have, need) = format_byte_pair(bytes, spec.minimum_free_bytes);
            check(
                PreflightStatus::Error,
                "Free space",
                format!("need at least {need} free before generation, have {have}"),
            )
        }
        Some(bytes) => check(
            PreflightStatus::Ok,
            "Free space",
            format!("{} available", format_bytes(bytes)),
        ),
        None => check(
            PreflightStatus::Warning,
            "Free space",
            "could not determine available disk space",
        ),
    });

    if !hardware.is_detected() {
        // The startup placeholder reports 0 bytes RAM and no GPUs; comparing
        // requirements against it would reject every machine.
        checks.push(check(
            PreflightStatus::Error,
            "Hardware",
            "hardware detection is still in progress; RAM and GPU requirements are checked once it finishes",
        ));
    } else {
        checks.push(system_memory_check(&entry, hardware.memory.total_bytes));
    }

    if let Some(required_vram) = entry.min_vram_bytes.filter(|_| hardware.is_detected()) {
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
                vram_shortfall_detail(max_vram, required_vram),
            )
        } else {
            check(
                PreflightStatus::Ok,
                "GPU memory",
                format!("{} dedicated VRAM available", format_bytes(max_vram)),
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

    validate_model_package(root, &entry, &mut checks, integrity)?;

    Ok(Some(MediaPreflightReport {
        model_name: entry.name,
        checks,
    }))
}

fn validate_model_package(
    root: &PortableRootManager,
    entry: &ModelCatalogEntry,
    checks: &mut Vec<PreflightCheck>,
    integrity: IntegrityPolicy,
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
    if primary_manifest_available(&manifest_path) {
        match read_primary_manifest(root, entry, &primary_path) {
            Ok(manifest) => validate_primary_manifest(
                root,
                entry,
                &model_dir,
                &primary_path,
                &manifest,
                checks,
                integrity,
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

    let hashes_checked = integrity == IntegrityPolicy::VerifyContents;
    if validate_installed_dependencies(root, entry, hashes_checked)? {
        let has_required = download.dependencies.iter().any(|dependency| dependency.required);
        checks.push(if !hashes_checked && has_required {
            check(
                PreflightStatus::Warning,
                "Dependencies",
                "required dependency paths/sizes match their manifest, but file content SHA-256 was not verified in this quick diagnostic; generation rechecks it",
            )
        } else {
            check(
                PreflightStatus::Ok,
                "Dependencies",
                if has_required {
                    "current dependency file SHA-256 matches recorded install digests"
                } else {
                    "no required dependency files"
                },
            )
        });
    } else {
        checks.push(check(
            PreflightStatus::Error,
            "Dependencies",
            "required package dependencies are missing, corrupt, or invalid; validate or re-download the model package",
        ));
    }

    Ok(())
}

fn validate_primary_manifest(
    root: &PortableRootManager,
    entry: &ModelCatalogEntry,
    model_dir: &Path,
    primary_path: &Path,
    manifest: &QwenModelManifest,
    checks: &mut Vec<PreflightCheck>,
    integrity: IntegrityPolicy,
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
    if !same_primary
        || manifest.size_bytes == 0
        || fs::metadata(primary_path)?.len() != manifest.size_bytes
    {
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

    if integrity == IntegrityPolicy::MetadataOnly {
        checks.push(check(
            PreflightStatus::Warning,
            "Model package",
            "primary model path, size and install metadata match, but current file content SHA-256 was not verified in this quick diagnostic; generation rechecks it",
        ));
        return Ok(());
    }

    let actual_digest = sha256_file(primary_path)?;
    if !manifest
        .actual_sha256
        .as_deref()
        .is_some_and(|recorded| actual_digest.eq_ignore_ascii_case(recorded))
        || manifest
            .sha256
            .as_deref()
            .is_some_and(|expected| !actual_digest.eq_ignore_ascii_case(expected))
    {
        checks.push(check(
            PreflightStatus::Error,
            "Model package",
            "primary model SHA-256 does not match its install manifest; file may be corrupted or replaced, re-download it",
        ));
        return Ok(());
    }
    checks.push(if manifest.verification == VerificationState::Verified {
        check(
            PreflightStatus::Ok,
            "Model package",
            "current primary model file SHA-256 matches its verified install manifest",
        )
    } else {
        check(
            PreflightStatus::Warning,
            "Model package",
            "current primary file SHA-256 matches the recorded local digest; no verified upstream checksum was available",
        )
    });
    Ok(())
}

fn system_memory_check(entry: &ModelCatalogEntry, detected_bytes: u64) -> PreflightCheck {
    if detected_bytes >= entry.min_ram_bytes {
        return check(
            PreflightStatus::Ok,
            "System memory",
            format!("{} RAM detected", format_bytes(detected_bytes)),
        );
    }

    let tolerated_floor = entry
        .min_ram_bytes
        .saturating_mul(100 - RAM_RESERVATION_TOLERANCE_PERCENT)
        / 100;
    let (detected, required) = format_byte_pair(detected_bytes, entry.min_ram_bytes);
    if detected_bytes >= tolerated_floor {
        return check(
            PreflightStatus::Warning,
            "System memory",
            format!(
                "{detected} RAM detected, slightly below the {required} catalog threshold; generation is allowed because OS-reported physical memory can exclude small hardware-reserved regions"
            ),
        );
    }

    check(
        PreflightStatus::Error,
        "System memory",
        format!(
            "{} requires at least {required} RAM; detected {detected}",
            entry.name
        ),
    )
}

/// Explains a VRAM shortfall with enough precision to be credible: 7.98 GiB is
/// "slightly below" 8.00 GiB, never "8.0 GiB is below 8.0 GiB".
fn vram_shortfall_detail(available: u64, recommended: u64) -> String {
    let (have, want) = format_byte_pair(available, recommended);
    let near = available.saturating_mul(100)
        >= recommended.saturating_mul(VRAM_NEAR_RECOMMENDATION_PERCENT);
    let degree = if near { "slightly below" } else { "below" };
    format!(
        "{have} dedicated VRAM is {degree} the {want} recommendation; CPU/offload fallback may be much slower"
    )
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
            detection_complete: true,
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
    fn startup_placeholder_is_not_judged_as_zero_ram() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let mut placeholder = hardware(0);
        placeholder.detection_complete = false;
        let report = preflight_with_environment(
            &root,
            "image",
            &placeholder,
            Some(100 * GIB),
            "windows",
            "x86_64",
        )
        .unwrap()
        .unwrap();
        let error = report.ensure_ready().unwrap_err().to_string();
        assert!(
            error.contains("hardware detection is still in progress"),
            "{error}"
        );
        assert!(!error.contains("System memory"), "{error}");
        assert!(!error.contains("GPU memory"), "{error}");
        assert!(!error.contains("0 bytes"), "{error}");
    }

    #[test]
    fn vram_messages_never_print_equal_values_for_a_shortfall() {
        let recommended = 8 * GIB;
        let at = |gib: f64| (gib * GIB as f64) as u64;

        let slightly = vram_shortfall_detail(at(7.98), recommended);
        assert!(
            slightly.starts_with("7.98 GiB dedicated VRAM is slightly below the 8.00 GiB"),
            "{slightly}"
        );

        let near = vram_shortfall_detail(at(7.94), recommended);
        assert!(
            near.starts_with("7.9 GiB dedicated VRAM is slightly below the 8.0 GiB"),
            "{near}"
        );

        let clearly = vram_shortfall_detail(at(7.5), recommended);
        assert!(
            clearly.starts_with("7.5 GiB dedicated VRAM is below the 8.0 GiB"),
            "{clearly}"
        );

        // The RX 580 from the live test reports 8,567,902,208 bytes.
        let rx580 = vram_shortfall_detail(8_567_902_208, recommended);
        assert!(
            rx580.contains("7.98 GiB") && rx580.contains("8.00 GiB"),
            "{rx580}"
        );
        assert!(rx580.contains("slightly below"), "{rx580}");
    }

    #[test]
    fn vram_comparison_uses_raw_bytes_at_the_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let required = entry_by_id("sdxl-base-1").unwrap().min_vram_bytes.unwrap();
        let gpu_check = |vram: u64| {
            let mut profile = hardware(32 * GIB);
            profile.gpus = vec![crate::hardware::GpuInfo {
                id: "gpu".to_string(),
                name: "test".to_string(),
                vendor: crate::hardware::GpuVendor::Amd,
                vendor_id: None,
                device_id: None,
                subsystem_id: None,
                revision: None,
                dedicated_vram_bytes: Some(vram),
                dedicated_system_memory_bytes: None,
                shared_memory_bytes: None,
                luid: None,
                is_discrete: true,
                is_integrated: false,
                is_software: false,
                available_backends: Vec::new(),
                recommended_backend: crate::hardware::BackendKind::Vulkan,
            }];
            preflight_with_environment(
                &root,
                "image",
                &profile,
                Some(100 * GIB),
                "windows",
                "x86_64",
            )
            .unwrap()
            .unwrap()
            .checks
            .into_iter()
            .find(|check| check.label == "GPU memory")
            .unwrap()
        };

        assert_eq!(gpu_check(required - 1).status, PreflightStatus::Warning);
        assert_eq!(gpu_check(required).status, PreflightStatus::Ok);
        assert_eq!(gpu_check(required + 10 * MIB).status, PreflightStatus::Ok);
        assert!(gpu_check(required)
            .detail
            .starts_with("8.0 GiB dedicated VRAM available"));
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
    fn near_threshold_ram_shortfall_is_warning_not_blocker() {
        let entry = entry_by_id("sdxl-base-1").unwrap();
        let detected = entry.min_ram_bytes - 256 * MIB;
        let memory_check = system_memory_check(&entry, detected);

        assert_eq!(memory_check.status, PreflightStatus::Warning);
        assert!(memory_check.detail.contains("hardware-reserved"));
    }

    #[test]
    fn material_ram_shortfall_remains_blocking() {
        let entry = entry_by_id("sdxl-base-1").unwrap();
        let memory_check = system_memory_check(&entry, 8 * GIB);

        assert_eq!(memory_check.status, PreflightStatus::Error);
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
    #[test]
    fn media_preflight_repairs_metadata_without_accepting_corrupted_model_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(dir.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let entry = entry_by_id("sdxl-base-1").unwrap();
        let download = entry.download.as_ref().unwrap();
        let model_dir = root.resolve_relative(&download.destination_dir).unwrap();
        fs::create_dir_all(&model_dir).unwrap();
        let primary = model_dir.join(&download.filename_pattern);
        fs::write(&primary, b"fixture-model").unwrap();
        let digest = crate::model_download::sha256_file(&primary).unwrap();
        let record = QwenModelManifest {
            repo: entry.repo.clone(),
            repo_sha: Some("a".repeat(40)),
            quantization: entry.quantization.clone(),
            filename: download.filename_pattern.clone(),
            size_bytes: 13,
            sha256: Some(digest.clone()),
            actual_sha256: Some(digest),
            verification: VerificationState::Verified,
            architecture: None,
            context_length: None,
            chat_template_available: false,
            source_url: "https://example.invalid/fixture".into(),
            installed_at: "fixture".into(),
        };
        let path = model_dir.join("model-manifest.json");
        fs::write(
            path.with_extension("json.recovery"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        for missing in [false, true] {
            if missing {
                fs::remove_file(&path).unwrap();
            } else {
                fs::write(&path, b"{truncated").unwrap();
            }
            let report = preflight_with_environment(
                &root,
                "image",
                &hardware(32 * GIB),
                Some(100 * GIB),
                "windows",
                "x86_64",
            )
            .unwrap()
            .unwrap();
            assert!(
                report
                    .checks
                    .iter()
                    .any(|check| check.label == "Model package"
                        && check.status == PreflightStatus::Ok)
            );
            assert!(serde_json::from_slice::<QwenModelManifest>(&fs::read(&path).unwrap()).is_ok());
        }
        fs::write(&primary, b"corrupt-model").unwrap();
        fs::write(&path, b"{truncated").unwrap();
        let report = preflight_with_environment(
            &root,
            "image",
            &hardware(32 * GIB),
            Some(100 * GIB),
            "windows",
            "x86_64",
        )
        .unwrap()
        .unwrap();
        assert!(report
            .ensure_ready()
            .unwrap_err()
            .to_string()
            .contains("Model package"));
        assert_eq!(fs::read(&path).unwrap(), b"{truncated");
    }
    #[test]
    fn media_preflight_detects_same_length_primary_and_dependency_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(dir.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let entry = entry_by_id("kokoro-82m-onnx").unwrap();
        let download = entry.download.as_ref().unwrap();
        let model_dir = root.resolve_relative(&download.destination_dir).unwrap();
        fs::create_dir_all(&model_dir).unwrap();

        let primary = model_dir.join("kokoro-v1_0.pth");
        let primary_bytes = b"primary-good";
        fs::write(&primary, primary_bytes).unwrap();
        let primary_digest = sha256_file(&primary).unwrap();
        let primary_record = QwenModelManifest {
            repo: entry.repo.clone(),
            repo_sha: Some("a".repeat(40)),
            quantization: entry.quantization.clone(),
            filename: "kokoro-v1_0.pth".into(),
            size_bytes: primary_bytes.len() as u64,
            sha256: Some(primary_digest.clone()),
            actual_sha256: Some(primary_digest),
            verification: VerificationState::Verified,
            architecture: None,
            context_length: None,
            chat_template_available: false,
            source_url: "https://example.invalid/model".into(),
            installed_at: "fixture".into(),
        };
        fs::write(
            model_dir.join("model-manifest.json"),
            serde_json::to_vec(&primary_record).unwrap(),
        )
        .unwrap();

        let mut dependency_files = Vec::new();
        for (role, filename, format, contents) in [
            ("config", "config.json", "json", &b"config-good"[..]),
            ("voice", "af_heart.pt", "pt", &b"voice-good"[..]),
        ] {
            let target = model_dir.join(filename);
            fs::write(&target, contents).unwrap();
            let digest = sha256_file(&target).unwrap();
            dependency_files.push(serde_json::json!({
                "role": role,
                "filename": filename,
                "format": format,
                "sizeBytes": contents.len(),
                "sha256": digest,
                "actualSha256": digest,
                "verification": "verified",
                "sourceUrl": "https://example.invalid/dependency"
            }));
        }
        fs::write(
            model_dir.join("package-manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "modelId": entry.id,
                "repo": entry.repo,
                "files": dependency_files
            }))
            .unwrap(),
        )
        .unwrap();

        let report = |integrity| {
            preflight_with_environment_and_policy(
                &root,
                "audio",
                &hardware(64 * GIB),
                Some(100 * GIB),
                "windows",
                "x86_64",
                integrity,
            )
            .unwrap()
            .unwrap()
        };
        let verified = report(IntegrityPolicy::VerifyContents);
        assert!(verified.ensure_ready().is_ok(), "{:?}", verified.checks);
        assert!(verified.issue_summary().is_none(), "{:?}", verified.checks);

        let quick = report(IntegrityPolicy::MetadataOnly);
        assert!(quick.ensure_ready().is_ok());
        assert!(quick.issue_summary().unwrap().contains("not verified"));

        // No size, path, or stored manifest hash changed.
        fs::write(&primary, b"primary-baad").unwrap();
        assert!(report(IntegrityPolicy::MetadataOnly).ensure_ready().is_ok());
        let primary_error = report(IntegrityPolicy::VerifyContents)
            .ensure_ready()
            .unwrap_err()
            .to_string();
        assert!(primary_error.contains("primary model SHA-256"), "{primary_error}");

        // Restoring the primary must not hide corruption of a required asset.
        fs::write(&primary, primary_bytes).unwrap();
        fs::write(model_dir.join("config.json"), b"config-baad").unwrap();
        assert!(report(IntegrityPolicy::MetadataOnly).ensure_ready().is_ok());
        let dependency_error = report(IntegrityPolicy::VerifyContents)
            .ensure_ready()
            .unwrap_err()
            .to_string();
        assert!(dependency_error.contains("Dependencies"), "{dependency_error}");
        assert!(dependency_error.contains("corrupt"), "{dependency_error}");
    }

    #[test]
    fn quick_media_diagnostic_never_claims_file_contents_were_verified() {
        let dir = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(dir.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let entry = entry_by_id("sdxl-base-1").unwrap();
        let download = entry.download.as_ref().unwrap();
        let model_dir = root.resolve_relative(&download.destination_dir).unwrap();
        fs::create_dir_all(&model_dir).unwrap();
        let primary = model_dir.join(&download.filename_pattern);
        fs::write(&primary, b"tiny-model").unwrap();
        let digest = sha256_file(&primary).unwrap();
        fs::write(
            model_dir.join("model-manifest.json"),
            serde_json::to_vec(&QwenModelManifest {
                repo: entry.repo.clone(),
                repo_sha: Some("b".repeat(40)),
                quantization: entry.quantization.clone(),
                filename: download.filename_pattern.clone(),
                size_bytes: 10,
                sha256: Some(digest.clone()),
                actual_sha256: Some(digest),
                verification: VerificationState::Verified,
                architecture: None,
                context_length: None,
                chat_template_available: false,
                source_url: "https://example.invalid/model".into(),
                installed_at: "fixture".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let quick = preflight_with_environment_and_policy(
            &root,
            "image",
            &hardware(64 * GIB),
            Some(100 * GIB),
            "windows",
            "x86_64",
            IntegrityPolicy::MetadataOnly,
        )
        .unwrap()
        .unwrap();
        let primary_check = quick
            .checks
            .iter()
            .find(|check| check.label == "Model package")
            .unwrap();
        assert_eq!(primary_check.status, PreflightStatus::Warning);
        assert!(primary_check.detail.contains("not verified"));
        assert!(quick.ensure_ready().is_ok());
    }
}
