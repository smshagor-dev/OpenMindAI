#[cfg(any(target_os = "android", target_os = "ios"))]
use serde::Serialize;
use tauri::State;

use crate::{app_error::AppError, AppState};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MobileTaskRoute {
    task: String,
    execution: String,
    local: bool,
    network_required: bool,
    model_id: Option<String>,
    model_name: Option<String>,
    installed: bool,
    supported: bool,
    reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MobileCapabilityReport {
    target: String,
    routes: Vec<MobileTaskRoute>,
    intentional_exclusions: Vec<String>,
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn catalog_route(
    state: &AppState,
    task: &str,
    model_id: &str,
    execution: &str,
    network_required: bool,
    reason: &str,
) -> MobileTaskRoute {
    match crate::installed_catalog_entry_by_id(state, model_id) {
        Ok(status) => {
            let installed = status.is_some();
            let name = status
                .as_ref()
                .map(|value| value.entry.name.clone())
                .or_else(|| crate::model_catalog::entry_by_id(model_id).ok().map(|value| value.name));
            MobileTaskRoute {
                task: task.to_string(),
                execution: execution.to_string(),
                local: execution.starts_with("local"),
                network_required,
                model_id: Some(model_id.to_string()),
                model_name: name,
                installed,
                supported: true,
                reason: if installed {
                    reason.to_string()
                } else {
                    format!("{reason} Required model package is not installed yet.")
                },
            }
        }
        Err(error) => MobileTaskRoute {
            task: task.to_string(),
            execution: execution.to_string(),
            local: execution.starts_with("local"),
            network_required,
            model_id: Some(model_id.to_string()),
            model_name: None,
            installed: false,
            supported: true,
            reason: format!("{reason} {error}"),
        },
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn text_route(state: &AppState, task: &str, network_required: bool) -> MobileTaskRoute {
    let preferred_reasoning = matches!(task, "thinking" | "math")
        .then(|| crate::installed_catalog_entry_by_id(state, "deepseek-r1-15b-q4km").ok().flatten())
        .flatten();
    if let Some(status) = preferred_reasoning {
        return MobileTaskRoute {
            task: task.to_string(),
            execution: if network_required {
                "local-model+network-retrieval".to_string()
            } else {
                "local-model".to_string()
            },
            local: true,
            network_required,
            model_id: Some(status.entry.id),
            model_name: Some(status.entry.name),
            installed: true,
            supported: true,
            reason: "Dedicated lightweight reasoning model is installed and preferred for this task."
                .to_string(),
        };
    }

    match crate::mobile_model_policy::recommendation_for_state(state) {
        Ok(recommendation) => MobileTaskRoute {
            task: task.to_string(),
            execution: if network_required {
                "local-model+network-retrieval".to_string()
            } else {
                "local-model".to_string()
            },
            local: true,
            network_required,
            model_id: Some(recommendation.model_id),
            model_name: Some(recommendation.name),
            installed: recommendation.installed,
            supported: true,
            reason: recommendation.reason,
        },
        Err(error) => MobileTaskRoute {
            task: task.to_string(),
            execution: "local-model".to_string(),
            local: true,
            network_required,
            model_id: None,
            model_name: None,
            installed: false,
            supported: true,
            reason: error.to_string(),
        },
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn route_task(state: &AppState, task: &str) -> MobileTaskRoute {
    match task.trim().to_ascii_lowercase().as_str() {
        "chat" | "writing" | "code" | "document" | "pdf" => text_route(state, task, false),
        "thinking" | "math" => text_route(state, task, false),
        "search" | "research" => text_route(state, task, true),
        "vision" | "ocr" | "image-review" | "pdf-vision" | "video-frame-review" => catalog_route(
            state,
            task,
            "qwen25-vl-3b-q4km",
            "local-native-multimodal",
            false,
            "OpenMindAI Lens handles screenshots, photos, rendered PDF pages, charts, OCR and representative video frames.",
        ),
        "speech-to-text" | "audio-transcription" => MobileTaskRoute {
            task: task.to_string(),
            execution: "system-or-connected-speech".to_string(),
            local: false,
            network_required: false,
            model_id: Some("whisper-large-v3-turbo-q5".to_string()),
            model_name: Some("OpenMindAI Hear".to_string()),
            installed: crate::installed_catalog_entry_by_id(state, "whisper-large-v3-turbo-q5")
                .ok()
                .flatten()
                .is_some(),
            supported: true,
            reason: "The desktop Whisper package remains available in the catalog; mobile uses the platform speech service when available until the native audio model path is enabled."
                .to_string(),
        },
        "text-to-speech" | "read-aloud" => MobileTaskRoute {
            task: task.to_string(),
            execution: "system-speech".to_string(),
            local: true,
            network_required: false,
            model_id: None,
            model_name: Some("Device speech synthesizer".to_string()),
            installed: true,
            supported: true,
            reason: "Mobile can read assistant responses aloud through the OS speech synthesizer without spawning a desktop runtime."
                .to_string(),
        },
        "image-generation" => MobileTaskRoute {
            task: task.to_string(),
            execution: "connected-or-remote-compute".to_string(),
            local: false,
            network_required: true,
            model_id: Some("sdxl-base-1".to_string()),
            model_name: Some("OpenMindAI Canvas".to_string()),
            installed: crate::installed_catalog_entry_by_id(state, "sdxl-base-1")
                .ok()
                .flatten()
                .is_some(),
            supported: true,
            reason: "SDXL-class generation is too memory-heavy for the default mobile runtime. Keep the feature in the app, but route it to capable connected/remote compute instead of crashing low-memory phones."
                .to_string(),
        },
        "video-generation" => MobileTaskRoute {
            task: task.to_string(),
            execution: "connected-or-remote-compute".to_string(),
            local: false,
            network_required: true,
            model_id: Some("wan21-t2v-13b".to_string()),
            model_name: Some("OpenMindAI Motion".to_string()),
            installed: false,
            supported: true,
            reason: "Video generation remains visible but uses connected/remote compute on mobile because the native package exceeds practical phone memory budgets."
                .to_string(),
        },
        "connected-apps" => MobileTaskRoute {
            task: task.to_string(),
            execution: "connected-native-actions".to_string(),
            local: false,
            network_required: true,
            model_id: None,
            model_name: None,
            installed: true,
            supported: true,
            reason: "Google, GitHub and configured connected-service actions remain available through explicit native connector commands."
                .to_string(),
        },
        other => MobileTaskRoute {
            task: other.to_string(),
            execution: "unsupported".to_string(),
            local: false,
            network_required: false,
            model_id: None,
            model_name: None,
            installed: false,
            supported: false,
            reason: "No mobile capability route is defined for this task yet.".to_string(),
        },
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub(crate) fn mobile_route_task(
    task: String,
    state: State<'_, AppState>,
) -> MobileTaskRoute {
    route_task(&state, &task)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub(crate) fn mobile_route_task(
    task: String,
    _state: State<'_, AppState>,
) -> MobileTaskRoute {
    MobileTaskRoute {
        task,
        execution: "desktop".to_string(),
        local: true,
        network_required: false,
        model_id: None,
        model_name: None,
        installed: true,
        supported: false,
        reason: "Mobile task routing is only used by Android/iOS builds.".to_string(),
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub(crate) fn mobile_capability_report(
    state: State<'_, AppState>,
) -> MobileCapabilityReport {
    let tasks = [
        "chat",
        "thinking",
        "code",
        "search",
        "research",
        "vision",
        "ocr",
        "pdf",
        "video-frame-review",
        "speech-to-text",
        "text-to-speech",
        "image-generation",
        "video-generation",
        "connected-apps",
    ];
    MobileCapabilityReport {
        target: if cfg!(target_os = "android") {
            "android".to_string()
        } else {
            "ios".to_string()
        },
        routes: tasks
            .iter()
            .map(|task| route_task(&state, task))
            .collect(),
        intentional_exclusions: vec![
            "autonomous agent execution".to_string(),
            "unrestricted filesystem access".to_string(),
            "full PC terminal/shell execution".to_string(),
        ],
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub(crate) fn mobile_capability_report(
    _state: State<'_, AppState>,
) -> MobileCapabilityReport {
    MobileCapabilityReport {
        target: "desktop".to_string(),
        routes: Vec::new(),
        intentional_exclusions: Vec::new(),
    }
}

pub(crate) fn _validate_router_contract() -> Result<(), AppError> {
    Ok(())
}
