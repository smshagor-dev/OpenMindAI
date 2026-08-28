use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    app_error::AppError, hardware::HardwareProfile, model_registry::ModelRecord,
    portable_root::PortableRootManager,
};

const CATALOG_JSON: &str = include_str!("../model-catalog.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub catalog_version: u32,
    pub models: Vec<ModelCatalogEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub family: String,
    pub kind: String,
    pub runtime: String,
    pub repo: String,
    pub quantization: String,
    pub required: bool,
    pub capabilities: Vec<String>,
    pub size_bytes: u64,
    pub min_ram_bytes: u64,
    pub min_vram_bytes: Option<u64>,
    pub license: String,
    pub description: String,
    pub download: Option<ModelCatalogDownload>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogDownload {
    pub strategy: ModelDownloadStrategy,
    pub filename_pattern: String,
    pub destination_dir: String,
    pub format: String,
    #[serde(default)]
    pub dependencies: Vec<ModelCatalogDependency>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogDependency {
    pub role: String,
    #[serde(default)]
    pub repo: Option<String>,
    pub filename_pattern: String,
    pub format: String,
    #[serde(default = "default_required_dependency")]
    pub required: bool,
}

fn default_required_dependency() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ModelDownloadStrategy {
    SingleFile,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogStatus {
    pub entry: ModelCatalogEntry,
    pub installed: bool,
    pub compatible: bool,
    pub download_supported: bool,
    pub installed_path: Option<String>,
    pub update_available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogReport {
    pub entries: Vec<ModelCatalogStatus>,
}

pub fn load_catalog() -> Result<ModelCatalog, AppError> {
    serde_json::from_str(CATALOG_JSON).map_err(|error| AppError::internal(error.to_string()))
}

pub fn required_entry() -> Result<ModelCatalogEntry, AppError> {
    load_catalog()?
        .models
        .into_iter()
        .find(|entry| entry.required)
        .ok_or_else(|| AppError::internal("model catalog has no required entry"))
}

pub fn entry_by_id(id: &str) -> Result<ModelCatalogEntry, AppError> {
    load_catalog()?
        .models
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| AppError::ModelNotFound(id.to_string()))
}

pub fn check_model_updates(
    installed: &[ModelRecord],
    hardware: &HardwareProfile,
    root: &PortableRootManager,
) -> Result<ModelCatalogReport, AppError> {
    let catalog = load_catalog()?;
    let total_ram = hardware.memory.total_bytes;
    let max_vram = hardware
        .gpus
        .iter()
        .filter_map(|gpu| gpu.dedicated_vram_bytes)
        .max();

    let entries = catalog
        .models
        .into_iter()
        .map(|entry| {
            let installed_path = installed_path_for(&entry, installed, root);
            let compatible = total_ram >= entry.min_ram_bytes
                && match entry.min_vram_bytes {
                    Some(required) => max_vram
                        .map(|available| available >= required)
                        .unwrap_or(false),
                    None => true,
                };
            let download_supported = entry
                .download
                .as_ref()
                .is_some_and(|download| download.strategy == ModelDownloadStrategy::SingleFile);
            ModelCatalogStatus {
                entry,
                installed: installed_path.is_some(),
                compatible,
                download_supported,
                installed_path,
                update_available: false,
            }
        })
        .collect();

    Ok(ModelCatalogReport { entries })
}

fn installed_path_for(
    entry: &ModelCatalogEntry,
    installed: &[ModelRecord],
    root: &PortableRootManager,
) -> Option<String> {
    let download = entry.download.as_ref();
    let primary = installed
        .iter()
        .find(|model| {
            model.source_repository.as_deref() == Some(entry.repo.as_str())
                || (model.family.as_deref() == Some(entry.family.as_str())
                    && model.quantization.as_deref() == Some(entry.quantization.as_str()))
        })
        .map(|model| model.path.clone())
        .or_else(|| download.and_then(|download| find_downloaded_file(root, download)))?;

    if let Some(download) = download {
        let package_dir = root.resolve_relative(&download.destination_dir).ok()?;
        if download
            .dependencies
            .iter()
            .filter(|dependency| dependency.required)
            .any(|dependency| {
                find_file_matching(&package_dir, &dependency.filename_pattern).is_none()
            })
        {
            return None;
        }
    }

    Some(primary)
}

fn find_downloaded_file(
    root: &PortableRootManager,
    download: &ModelCatalogDownload,
) -> Option<String> {
    let dir = root.resolve_relative(&download.destination_dir).ok()?;
    if !dir.exists() {
        return None;
    }
    find_file_matching(&dir, &download.filename_pattern)
        .and_then(|path| path.strip_prefix(root.root()).ok().map(Path::to_path_buf))
        .map(|path| path.to_string_lossy().replace('\\', "/"))
}

pub(crate) fn installed_file_for_pattern(
    root: &PortableRootManager,
    destination_dir: &str,
    pattern: &str,
) -> Option<PathBuf> {
    let dir = root.resolve_relative(destination_dir).ok()?;
    if !dir.exists() {
        return None;
    }
    find_file_matching(&dir, pattern)
}

pub fn delete_catalog_model(root: &PortableRootManager, model_id: &str) -> Result<(), AppError> {
    let entry = entry_by_id(model_id)?;
    let download = entry.download.as_ref().ok_or_else(|| {
        AppError::ModelUnsupported(format!(
            "{} has no removable model data configured",
            entry.name
        ))
    })?;
    let target_dir = root.resolve_relative(&download.destination_dir)?;
    if !target_dir.exists() {
        return Ok(());
    }
    ensure_under_root(root, &target_dir)?;

    remove_matching_file(root, &target_dir, &download.filename_pattern)?;
    for dependency in &download.dependencies {
        remove_matching_file(root, &target_dir, &dependency.filename_pattern)?;
    }

    for manifest_name in ["model-manifest.json", "package-manifest.json"] {
        let manifest = target_dir.join(manifest_name);
        if manifest.exists() {
            ensure_under_root(root, &manifest)?;
            std::fs::remove_file(manifest)?;
        }
    }

    remove_empty_dirs_up_to(root.root(), target_dir)?;
    Ok(())
}

fn remove_matching_file(
    root: &PortableRootManager,
    target_dir: &Path,
    pattern: &str,
) -> Result<(), AppError> {
    if let Some(file) = find_file_matching(target_dir, pattern) {
        ensure_under_root(root, &file)?;
        std::fs::remove_file(file)?;
    }
    Ok(())
}

fn ensure_under_root(root: &PortableRootManager, path: &Path) -> Result<(), AppError> {
    let canonical_root = std::fs::canonicalize(root.root())?;
    let canonical_path = std::fs::canonicalize(path)?;
    if !canonical_path.starts_with(canonical_root) {
        return Err(AppError::ModelInvalid(
            "model path escapes OpenMindAI Root".to_string(),
        ));
    }
    Ok(())
}

fn remove_empty_dirs_up_to(root: &Path, mut dir: PathBuf) -> Result<(), AppError> {
    let canonical_root = std::fs::canonicalize(root)?;
    loop {
        if !dir.exists() {
            break;
        }
        let canonical_dir = std::fs::canonicalize(&dir)?;
        if canonical_dir == canonical_root || !canonical_dir.starts_with(&canonical_root) {
            break;
        }
        if std::fs::read_dir(&dir)?.next().is_some() {
            break;
        }
        std::fs::remove_dir(&dir)?;
        let Some(parent) = dir.parent() else {
            break;
        };
        dir = parent.to_path_buf();
    }
    Ok(())
}

fn find_file_matching(dir: &Path, pattern: &str) -> Option<std::path::PathBuf> {
    for entry in std::fs::read_dir(dir).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_file_matching(&path, pattern) {
                return Some(found);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| wildcard_match(pattern, name))
        {
            return Some(path);
        }
    }
    None
}

pub(crate) fn wildcard_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let pattern = pattern.to_ascii_lowercase();
    let value = value.to_ascii_lowercase();
    let parts = pattern.split('*').collect::<Vec<_>>();
    if parts.len() == 1 {
        return pattern == value;
    }

    let mut cursor = 0;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let Some(found) = value[cursor..].find(part) else {
            return false;
        };
        if index == 0 && found != 0 {
            return false;
        }
        cursor += found + part.len();
    }

    if pattern.ends_with('*') {
        true
    } else {
        parts.last().is_none_or(|last| value.ends_with(last))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{BackendKind, CpuProfile, GpuInfo, GpuVendor, MemoryProfile};

    #[test]
    fn bundled_catalog_parses_and_has_entries() {
        let catalog = load_catalog().unwrap();
        assert!(catalog.models.len() > 1);
        assert_eq!(catalog.models[0].family, "qwen");
        assert!(catalog.models.iter().any(|entry| entry.kind == "image"));
        assert!(catalog.models.iter().any(|entry| entry.kind == "video"));
        assert!(catalog
            .models
            .iter()
            .any(|entry| entry.kind == "speech-to-text"));
        let lens = catalog
            .models
            .iter()
            .find(|entry| entry.id == "qwen25-vl-3b-q4km")
            .unwrap();
        assert!(lens
            .download
            .as_ref()
            .unwrap()
            .dependencies
            .iter()
            .any(|dependency| dependency.role == "mmproj" && dependency.required));

        let speak = catalog
            .models
            .iter()
            .find(|entry| entry.id == "kokoro-82m-onnx")
            .unwrap();
        assert_eq!(speak.runtime, "any-tts-candle");
        assert!(speak
            .download
            .as_ref()
            .unwrap()
            .dependencies
            .iter()
            .any(|dependency| dependency.role == "voice" && dependency.required));

        let motion = catalog
            .models
            .iter()
            .find(|entry| entry.id == "wan21-t2v-13b")
            .unwrap();
        assert_eq!(motion.runtime, "stable-diffusion.cpp");
        assert!(motion
            .download
            .as_ref()
            .unwrap()
            .dependencies
            .iter()
            .any(|dependency| dependency.role == "text-encoder"
                && dependency.repo.as_deref() == Some("city96/umt5-xxl-encoder-gguf")));
    }

    fn hardware_with(total_ram_bytes: u64, vram_bytes: Option<u64>) -> HardwareProfile {
        HardwareProfile {
            operating_system: "windows".to_string(),
            architecture: "x86_64".to_string(),
            cpu: CpuProfile {
                name: "test-cpu".to_string(),
                physical_cores: Some(6),
                logical_threads: 12,
            },
            memory: MemoryProfile {
                total_bytes: total_ram_bytes,
                available_bytes: total_ram_bytes,
            },
            gpus: vram_bytes
                .map(|vram| {
                    vec![GpuInfo {
                        id: "gpu-0".to_string(),
                        name: "test-gpu".to_string(),
                        vendor: GpuVendor::Amd,
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
                        available_backends: vec![BackendKind::Vulkan],
                        recommended_backend: BackendKind::Vulkan,
                    }]
                })
                .unwrap_or_default(),
            primary_gpu: None,
            recommended_inference_gpu: None,
            backends: crate::hardware::BackendProfile {
                cpu: true,
                cuda: false,
                vulkan: vram_bytes.is_some(),
                sycl: false,
                hip: false,
                metal: false,
            },
        }
    }

    #[test]
    fn installed_catalog_model_never_reports_a_false_update() {
        let hardware = hardware_with(16 * 1024 * 1024 * 1024, Some(8 * 1024 * 1024 * 1024));
        let installed = vec![ModelRecord {
            id: "installed-1".to_string(),
            name: "Qwen3 4B".to_string(),
            family: Some("qwen".to_string()),
            path: "models/llm/qwen/qwen3-4b/model.gguf".to_string(),
            format: "gguf".to_string(),
            quantization: Some("Q4_K_M".to_string()),
            size_bytes: 2_497_280_256,
            capabilities: "[]".to_string(),
            context_length: None,
            preferred_backend: None,
            enabled: true,
            source_repository: None,
            verification: None,
            state: crate::model_registry::ModelLifecycleState::Ready,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }];
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();

        let report = check_model_updates(&installed, &hardware, &root).unwrap();

        assert!(report.entries.len() > 1);
        assert!(report.entries[0].installed);
        assert!(!report.entries[0].update_available);
    }

    #[test]
    fn flags_incompatible_hardware() {
        let low_ram_hardware = hardware_with(2 * 1024 * 1024 * 1024, None);
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();

        let report = check_model_updates(&[], &low_ram_hardware, &root).unwrap();

        assert!(!report.entries[0].compatible);
    }

    #[test]
    fn wildcard_patterns_match_download_files() {
        assert!(wildcard_match("*Q4_K_M*.gguf", "Qwen3-8B-Q4_K_M.gguf"));
        assert!(wildcard_match(
            "ggml-large-v3-turbo-q5_0.bin",
            "ggml-large-v3-turbo-q5_0.bin"
        ));
        assert!(wildcard_match(
            "mmproj-*Q8_0*.gguf",
            "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf"
        ));
        assert!(!wildcard_match("*quantized*.onnx", "model.onnx"));
    }
}
