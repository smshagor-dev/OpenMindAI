#[cfg(any(target_os = "android", target_os = "ios"))]
use std::path::Path;

#[cfg(any(target_os = "android", target_os = "ios"))]
use serde::Serialize;
use tauri::State;

use crate::{app_error::AppError, AppState};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MobilePreparedRoute {
    task: String,
    registry_model_id: String,
    model_name: String,
    reason: String,
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn registry_path_for_catalog_path(path: &str) -> String {
    let normalized = path.trim().replace('\\', "/");
    if normalized.starts_with("models/") {
        normalized
    } else {
        format!("models/{normalized}")
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn supports_text_task(model: &crate::model_registry::ModelRecord, task: &str) -> bool {
    if !model.enabled || !model.format.eq_ignore_ascii_case("gguf") {
        return false;
    }
    let capabilities = serde_json::from_str::<Vec<String>>(&model.capabilities).unwrap_or_default();
    let has = |name: &str| capabilities.iter().any(|value| value == name);
    match task {
        "thinking" | "math" => has("reasoning") || has("thinking") || has("chat"),
        "code" => has("code") || has("chat"),
        _ => has("chat"),
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn model_from_catalog(
    state: &AppState,
    catalog_id: &str,
    discovered: &[crate::model_registry::ModelRecord],
) -> Result<Option<crate::model_registry::ModelRecord>, AppError> {
    let Some(status) = crate::installed_catalog_entry_by_id(state, catalog_id)? else {
        return Ok(None);
    };
    let Some(path) = status.installed_path else {
        return Ok(None);
    };
    let expected = registry_path_for_catalog_path(&path);
    Ok(discovered
        .iter()
        .find(|model| model.path.replace('\\', "/") == expected)
        .cloned())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn prepare_route(
    state: &AppState,
    conversation_id: &str,
    task: &str,
) -> Result<MobilePreparedRoute, AppError> {
    use crate::{
        chat::ChatRepository,
        mobile_model_policy::recommendation_for_state,
        model_registry::ModelRegistry,
    };

    let normalized_task = task.trim().to_ascii_lowercase();
    if !matches!(
        normalized_task.as_str(),
        "chat"
            | "writing"
            | "code"
            | "thinking"
            | "math"
            | "search"
            | "research"
            | "document"
            | "pdf"
    ) {
        return Err(AppError::ModelUnsupported(format!(
            "{task} is not a text-model route"
        )));
    }

    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let chats = ChatRepository::new(&db);
    let conversation = chats.find_conversation(conversation_id)?;
    let registry = ModelRegistry::new(&db, &state.root);
    let discovered = registry.discover_gguf_models()?;

    if let Some(active_id) = conversation.active_model_id.as_deref() {
        if let Some(selected) = discovered.iter().find(|model| model.id == active_id) {
            if supports_text_task(selected, &normalized_task) {
                let validated = registry.validate_model(&selected.id)?;
                return Ok(MobilePreparedRoute {
                    task: normalized_task,
                    registry_model_id: validated.id,
                    model_name: validated.name,
                    reason: "Using the compatible local model explicitly selected for this conversation."
                        .to_string(),
                });
            }
        }
    }

    if matches!(normalized_task.as_str(), "thinking" | "math") {
        if let Some(reasoning) = model_from_catalog(state, "deepseek-r1-15b-q4km", &discovered)? {
            if supports_text_task(&reasoning, &normalized_task) {
                let validated = registry.validate_model(&reasoning.id)?;
                chats.set_active_model(conversation_id, Some(&validated.id))?;
                return Ok(MobilePreparedRoute {
                    task: normalized_task,
                    registry_model_id: validated.id,
                    model_name: validated.name,
                    reason: "Task router selected the installed lightweight reasoning model."
                        .to_string(),
                });
            }
        }
    }

    let recommendation = recommendation_for_state(state)?;
    let installed_path = recommendation.installed_model_path.ok_or_else(|| {
        AppError::ModelNotFound(format!(
            "{} is recommended for this device but is not installed",
            recommendation.name
        ))
    })?;
    let expected = format!(
        "models/{}",
        installed_path.trim().replace('\\', "/').trim_start_matches("models/")
    );
    let selected = discovered
        .iter()
        .find(|model| model.path.replace('\\', "/") == expected)
        .ok_or_else(|| {
            AppError::ModelNotFound(format!(
                "{} is installed but has not been registered yet",
                recommendation.name
            ))
        })?;
    if !supports_text_task(selected, &normalized_task) {
        return Err(AppError::ModelUnsupported(format!(
            "{} cannot serve the requested {normalized_task} task",
            selected.name
        )));
    }
    let validated = registry.validate_model(&selected.id)?;
    chats.set_active_model(conversation_id, Some(&validated.id))?;
    Ok(MobilePreparedRoute {
        task: normalized_task,
        registry_model_id: validated.id,
        model_name: validated.name,
        reason: format!(
            "Task router selected the device {} tier. {}",
            recommendation.tier, recommendation.reason
        ),
    })
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub(crate) fn mobile_prepare_text_route(
    conversation_id: String,
    task: String,
    state: State<'_, AppState>,
) -> Result<MobilePreparedRoute, AppError> {
    if conversation_id.trim().is_empty() {
        return Err(AppError::InferenceFailed(
            "conversation id is required for mobile model routing".to_string(),
        ));
    }
    prepare_route(&state, &conversation_id, &task)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub(crate) fn mobile_prepare_text_route(
    _conversation_id: String,
    _task: String,
    _state: State<'_, AppState>,
) -> Result<MobilePreparedRoute, AppError> {
    Err(AppError::ModelUnsupported(
        "mobile text routing is only available in Android/iOS builds".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_paths_are_normalized_under_models() {
        assert_eq!(
            registry_path_for_catalog_path("vision/qwen/model.gguf"),
            "models/vision/qwen/model.gguf"
        );
        assert_eq!(
            registry_path_for_catalog_path("models/llm/qwen/model.gguf"),
            "models/llm/qwen/model.gguf"
        );
    }

    #[test]
    fn task_capability_matching_prefers_specialized_metadata() {
        let model = crate::model_registry::ModelRecord {
            id: "test".to_string(),
            name: "test".to_string(),
            family: None,
            path: "models/test.gguf".to_string(),
            format: "gguf".to_string(),
            quantization: None,
            size_bytes: 1,
            capabilities: serde_json::to_string(&vec!["chat", "code", "reasoning"]).unwrap(),
            context_length: None,
            preferred_backend: None,
            enabled: true,
            source_repository: None,
            verification: None,
            state: crate::model_registry::ModelLifecycleState::Ready,
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert!(supports_text_task(&model, "code"));
        assert!(supports_text_task(&model, "thinking"));
        assert!(supports_text_task(&model, "chat"));
    }
}
