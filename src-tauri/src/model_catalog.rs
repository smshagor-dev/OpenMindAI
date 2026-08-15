use serde::{Deserialize, Serialize};

use crate::{app_error::AppError, hardware::HardwareProfile, model_registry::ModelRecord};

/// Source-controlled catalog, not fetched remotely — matches the spec's
/// "source-controlled catalog... optionally a remotely retrievable
/// production catalog" where the remote part is explicit future work.
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
    /// The Hugging Face repo this entry downloads from — the single source
    /// of truth `model_download.rs` reads instead of duplicating its own
    /// hardcoded repo constant (see `ModelDownloadManager::catalog_entry`).
    pub repo: String,
    pub quantization: String,
    pub required: bool,
    pub capabilities: Vec<String>,
    pub size_bytes: u64,
    pub min_ram_bytes: u64,
    pub min_vram_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogStatus {
    pub entry: ModelCatalogEntry,
    pub installed: bool,
    pub compatible: bool,
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

/// The catalog's required first-run model — v1 ships exactly one, so this is
/// unambiguous. `model_download.rs`'s automatic setup download reads the
/// repo/quantization to fetch from here rather than duplicating them as
/// separate hardcoded constants; once a second catalog entry ever exists,
/// callers of this function are exactly the call sites that need to grow
/// real model selection instead of assuming "the one required entry".
pub fn required_entry() -> Result<ModelCatalogEntry, AppError> {
    load_catalog()?
        .models
        .into_iter()
        .find(|entry| entry.required)
        .ok_or_else(|| AppError::internal("model catalog has no required entry"))
}

/// Cross-references the (source-controlled) catalog against what's actually
/// installed and this machine's hardware. v1's catalog has exactly one
/// entry, and it's the model the app already ships against, so
/// `update_available` is honestly always `false` today — not an
/// unimplemented check, just the correct answer until a second catalog
/// entry actually exists.
pub fn check_model_updates(
    installed: &[ModelRecord],
    hardware: &HardwareProfile,
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
            let installed_flag = installed.iter().any(|model| {
                model.family.as_deref() == Some(entry.family.as_str())
                    && model.quantization.as_deref() == Some(entry.quantization.as_str())
            });
            let compatible = total_ram >= entry.min_ram_bytes
                && match entry.min_vram_bytes {
                    Some(required) => max_vram
                        .map(|available| available >= required)
                        .unwrap_or(false),
                    None => true,
                };
            ModelCatalogStatus {
                entry,
                installed: installed_flag,
                compatible,
                update_available: false,
            }
        })
        .collect();

    Ok(ModelCatalogReport { entries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{BackendKind, CpuProfile, GpuInfo, GpuVendor, MemoryProfile};

    #[test]
    fn bundled_catalog_parses_and_has_one_entry() {
        let catalog = load_catalog().unwrap();
        assert_eq!(catalog.models.len(), 1);
        assert_eq!(catalog.models[0].family, "qwen");
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

        let report = check_model_updates(&installed, &hardware).unwrap();
        assert_eq!(report.entries.len(), 1);
        assert!(report.entries[0].installed);
        assert!(!report.entries[0].update_available);
    }

    #[test]
    fn flags_incompatible_hardware() {
        let low_ram_hardware = hardware_with(2 * 1024 * 1024 * 1024, None);
        let report = check_model_updates(&[], &low_ram_hardware).unwrap();
        assert!(!report.entries[0].compatible);
    }
}
