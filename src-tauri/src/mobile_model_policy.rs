use serde::Serialize;
use tauri::State;

use crate::{app_error::AppError, model_catalog, AppState};

const GIB: u64 = 1024 * 1024 * 1024;
const SWIFT_MIN_RAM: u64 = 6 * GIB;
const CORE_MIN_RAM: u64 = 12 * GIB;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileModelRecommendation {
    pub supported: bool,
    pub tier: &'static str,
    pub model_id: String,
    pub name: String,
    pub repository: String,
    pub quantization: String,
    pub size_bytes: u64,
    pub total_ram_bytes: u64,
    pub installed: bool,
    /// Path relative to the app-private `models/` directory.
    pub installed_model_path: Option<String>,
    pub reason: String,
}

fn recommended_model_id(total_ram: u64) -> (&'static str, &'static str, &'static str) {
    if total_ram >= CORE_MIN_RAM {
        (
            "core",
            "qwen3-4b-q4km",
            "12 GB or more RAM detected; Core offers the strongest mobile quality while keeping a conservative memory margin.",
        )
    } else if total_ram >= SWIFT_MIN_RAM {
        (
            "swift",
            "qwen3-17b-q4km",
            "6-12 GB RAM detected; Swift balances response quality, memory use, and sustained mobile thermals.",
        )
    } else {
        (
            "nano",
            "qwen3-06b-q4",
            "Less than 6 GB RAM detected; Nano minimizes memory pressure and is the safest local model for this device.",
        )
    }
}

pub(crate) fn recommendation_for_state(
    state: &AppState,
) -> Result<MobileModelRecommendation, AppError> {
    let total_ram = state.hardware.memory.total_bytes;
    let (tier, model_id, reason) = recommended_model_id(total_ram);
    let entry = model_catalog::entry_by_id(model_id)?;

    let installed_model_path = entry.download.as_ref().and_then(|download| {
        model_catalog::installed_file_for_pattern(
            &state.root,
            &download.destination_dir,
            &download.filename_pattern,
        )
        .and_then(|path| {
            path.strip_prefix(state.root.models_dir())
                .ok()
                .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        })
    });

    Ok(MobileModelRecommendation {
        supported: true,
        tier,
        model_id: entry.id,
        name: entry.name,
        repository: entry.repo,
        quantization: entry.quantization,
        size_bytes: entry.size_bytes,
        total_ram_bytes: total_ram,
        installed: installed_model_path.is_some(),
        installed_model_path,
        reason: reason.to_string(),
    })
}

#[tauri::command]
pub fn mobile_model_recommendation(
    state: State<'_, AppState>,
) -> Result<MobileModelRecommendation, AppError> {
    recommendation_for_state(&state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommends_nano_below_six_gib() {
        let (tier, id, _) = recommended_model_id(4 * GIB);
        assert_eq!(tier, "nano");
        assert_eq!(id, "qwen3-06b-q4");
    }

    #[test]
    fn recommends_swift_for_midrange_memory() {
        let (tier, id, _) = recommended_model_id(8 * GIB);
        assert_eq!(tier, "swift");
        assert_eq!(id, "qwen3-17b-q4km");
    }

    #[test]
    fn recommends_core_only_with_large_memory_margin() {
        let (tier, id, _) = recommended_model_id(12 * GIB);
        assert_eq!(tier, "core");
        assert_eq!(id, "qwen3-4b-q4km");
    }
}
