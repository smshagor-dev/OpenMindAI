#[cfg(any(target_os = "android", target_os = "ios"))]
use std::path::Path;

#[cfg(any(target_os = "android", target_os = "ios"))]
use tauri::Emitter;
use tauri::{AppHandle, State};

use crate::{app_error::AppError, chat::Message, AppState};

#[cfg(any(target_os = "android", target_os = "ios"))]
struct MobileModelSelection {
    registry_id: String,
    name: String,
    relative_model_path: String,
    reason: String,
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub async fn mobile_send_chat_message(
    app: AppHandle,
    conversation_id: String,
    content: String,
    mode: String,
    state: State<'_, AppState>,
    native: State<'_, crate::mobile_inference::MobileInferenceState>,
) -> Result<Message, AppError> {
    send_mobile_chat(app, conversation_id, content, mode, state, native).await
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub async fn mobile_send_chat_message(
    app: AppHandle,
    conversation_id: String,
    content: String,
    mode: String,
    state: State<'_, AppState>,
) -> Result<Message, AppError> {
    let _ = (app, conversation_id, content, mode, state);
    Err(AppError::ModelUnsupported(
        "on-device mobile chat is only available in OpenMindAI mobile builds".to_string(),
    ))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub async fn mobile_regenerate_message(
    app: AppHandle,
    conversation_id: String,
    assistant_message_id: String,
    mode: String,
    state: State<'_, AppState>,
    native: State<'_, crate::mobile_inference::MobileInferenceState>,
) -> Result<Message, AppError> {
    regenerate_mobile_chat(
        app,
        conversation_id,
        assistant_message_id,
        mode,
        state,
        native,
    )
    .await
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub async fn mobile_regenerate_message(
    app: AppHandle,
    conversation_id: String,
    assistant_message_id: String,
    mode: String,
    state: State<'_, AppState>,
) -> Result<Message, AppError> {
    let _ = (app, conversation_id, assistant_message_id, mode, state);
    Err(AppError::ModelUnsupported(
        "on-device mobile chat regeneration is only available in OpenMindAI mobile builds"
            .to_string(),
    ))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
async fn send_mobile_chat(
    app: AppHandle,
    conversation_id: String,
    content: String,
    mode: String,
    state: State<'_, AppState>,
    native: State<'_, crate::mobile_inference::MobileInferenceState>,
) -> Result<Message, AppError> {
    use crate::{
        chat::ChatRepository,
        inference::StreamStartedEvent,
        mobile_inference::{resolve_android_model_path, DEFAULT_MAX_OUTPUT_TOKENS},
    };

    validate_mobile_mode(&mode)?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AppError::InferenceFailed(
            "message cannot be empty".to_string(),
        ));
    }

    crate::sync_project_context(&state, &conversation_id)?;
    let selection = select_mobile_model(&state, &conversation_id, &mode)?;
    let (model_path, model_display) =
        resolve_android_model_path(&state, &selection.relative_model_path)?;
    let output_limit = output_limit_for_mode(&mode, DEFAULT_MAX_OUTPUT_TOKENS);
    let cancellation = state.active_generations.start(&conversation_id)?;

    let created = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"));
        db.and_then(|db| {
            let repo = ChatRepository::new(&db);
            repo.set_active_model(&conversation_id, Some(&selection.registry_id))?;
            let user = repo.add_message(
                &conversation_id,
                "user",
                trimmed,
                "completed",
                Some(&selection.registry_id),
            )?;
            let assistant = repo.add_message(
                &conversation_id,
                "assistant",
                "",
                "streaming",
                Some(&selection.registry_id),
            )?;
            Ok((user, assistant))
        })
    };

    let (user, assistant) = match created {
        Ok(value) => value,
        Err(error) => {
            state.active_generations.finish(&conversation_id);
            return Err(error);
        }
    };

    if let Err(error) = app.emit(
        "inference:started",
        StreamStartedEvent {
            conversation_id: conversation_id.clone(),
            user,
            assistant: assistant.clone(),
            routed_model_name: selection.name.clone(),
            routing_reason: selection.reason.clone(),
        },
    ) {
        mark_mobile_generation_failed(&state, &conversation_id, &assistant.id, &app);
        return Err(AppError::StreamFailed(error.to_string()));
    }

    run_mobile_completion(
        &app,
        &state,
        native.inner().clone(),
        &conversation_id,
        &assistant,
        model_path,
        model_display,
        output_limit,
        cancellation,
    )
    .await
}

#[cfg(any(target_os = "android", target_os = "ios"))]
async fn regenerate_mobile_chat(
    app: AppHandle,
    conversation_id: String,
    assistant_message_id: String,
    mode: String,
    state: State<'_, AppState>,
    native: State<'_, crate::mobile_inference::MobileInferenceState>,
) -> Result<Message, AppError> {
    use crate::{
        chat::ChatRepository,
        inference::StreamStartedEvent,
        mobile_inference::{resolve_android_model_path, DEFAULT_MAX_OUTPUT_TOKENS},
    };

    validate_mobile_mode(&mode)?;
    crate::sync_project_context(&state, &conversation_id)?;
    let selection = select_mobile_model(&state, &conversation_id, &mode)?;
    let (model_path, model_display) =
        resolve_android_model_path(&state, &selection.relative_model_path)?;
    let output_limit = output_limit_for_mode(&mode, DEFAULT_MAX_OUTPUT_TOKENS);
    let cancellation = state.active_generations.start(&conversation_id)?;

    let created = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"));
        db.and_then(|db| {
            let repo = ChatRepository::new(&db);
            let history = repo.list_messages(&conversation_id)?;
            let target_index = history
                .iter()
                .position(|message| message.id == assistant_message_id)
                .ok_or_else(|| AppError::internal("assistant message not found"))?;
            let user = history[..target_index]
                .iter()
                .rev()
                .find(|message| message.role == "user")
                .cloned()
                .ok_or_else(|| {
                    AppError::internal("no preceding user message to regenerate from")
                })?;
            if user.content.contains("[Mode: Image/Vision Review]")
                || user.content.contains("[Mode: Multimodal Vision Review]")
            {
                return Err(AppError::InferenceFailed(
                    "Vision responses cannot be regenerated by the mobile text-only local runtime. Reattach the image and send it again."
                        .to_string(),
                ));
            }

            repo.delete_message(&assistant_message_id)?;
            repo.set_active_model(&conversation_id, Some(&selection.registry_id))?;
            let assistant = repo.add_message(
                &conversation_id,
                "assistant",
                "",
                "streaming",
                Some(&selection.registry_id),
            )?;
            Ok((user, assistant))
        })
    };

    let (user, assistant) = match created {
        Ok(value) => value,
        Err(error) => {
            state.active_generations.finish(&conversation_id);
            return Err(error);
        }
    };

    if let Err(error) = app.emit(
        "inference:started",
        StreamStartedEvent {
            conversation_id: conversation_id.clone(),
            user,
            assistant: assistant.clone(),
            routed_model_name: selection.name.clone(),
            routing_reason: selection.reason.clone(),
        },
    ) {
        mark_mobile_generation_failed(&state, &conversation_id, &assistant.id, &app);
        return Err(AppError::StreamFailed(error.to_string()));
    }

    run_mobile_completion(
        &app,
        &state,
        native.inner().clone(),
        &conversation_id,
        &assistant,
        model_path,
        model_display,
        output_limit,
        cancellation,
    )
    .await
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn select_mobile_model(
    state: &AppState,
    conversation_id: &str,
    mode: &str,
) -> Result<MobileModelSelection, AppError> {
    use crate::{
        chat::ChatRepository, mobile_model_policy::recommendation_for_state,
        model_registry::ModelRegistry,
    };

    let recommendation = recommendation_for_state(state)?;
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let conversation = ChatRepository::new(&db).find_conversation(conversation_id)?;
    let registry = ModelRegistry::new(&db, &state.root);
    let discovered = registry.discover_gguf_models()?;

    if let Some(active_id) = conversation.active_model_id.as_deref() {
        if let Some(selected) = discovered.iter().find(|model| model.id == active_id) {
            ensure_mobile_model_supports_mode(selected, mode)?;
            let validated = registry.validate_model(&selected.id)?;
            return Ok(MobileModelSelection {
                registry_id: validated.id,
                name: validated.name,
                relative_model_path: relative_path_below_models(&validated.path)?,
                reason: "Using the installed local model selected for this conversation."
                    .to_string(),
            });
        }
        // Older mobile builds persisted a catalog id instead of the registry
        // model id. Treat that stale value as no explicit selection so the
        // device recommendation can repair the conversation automatically.
    }

    let recommended_relative = recommendation.installed_model_path.clone().ok_or_else(|| {
        AppError::ModelNotFound(format!(
            "{} is recommended for this mobile device but is not installed. Install it from the mobile setup or Models screen first.",
            recommendation.name
        ))
    })?;
    let expected_registry_path = format!("models/{}", recommended_relative.replace('\\', "/"));
    let selected = discovered
        .iter()
        .find(|model| model.path.replace('\\', "/") == expected_registry_path)
        .ok_or_else(|| {
            AppError::ModelNotFound(format!(
                "the installed {} package is not registered yet; reopen Models and retry",
                recommendation.name
            ))
        })?;
    ensure_mobile_model_supports_mode(selected, mode)?;
    let validated = registry.validate_model(&selected.id)?;

    Ok(MobileModelSelection {
        registry_id: validated.id,
        name: validated.name,
        relative_model_path: relative_path_below_models(&validated.path)?,
        reason: format!(
            "Mobile on-device {} tier selected from detected RAM. {}",
            recommendation.tier, recommendation.reason
        ),
    })
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn relative_path_below_models(registry_path: &str) -> Result<String, AppError> {
    let normalized = registry_path.trim().replace('\\', "/");
    let relative = normalized
        .strip_prefix("models/")
        .ok_or_else(|| {
            AppError::ModelUnsupported(
                "mobile local models must stay inside the app-private models directory".to_string(),
            )
        })?
        .trim();
    if relative.is_empty()
        || Path::new(relative).is_absolute()
        || Path::new(relative)
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(AppError::ModelUnsupported(
            "invalid mobile model path".to_string(),
        ));
    }
    Ok(relative.to_string())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn ensure_mobile_model_supports_mode(
    model: &crate::model_registry::ModelRecord,
    mode: &str,
) -> Result<(), AppError> {
    if !model.enabled {
        return Err(AppError::ModelUnsupported(format!(
            "{} is disabled and cannot be used for mobile inference",
            model.name
        )));
    }
    if !model.format.eq_ignore_ascii_case("gguf") {
        return Err(AppError::ModelUnsupported(format!(
            "{} is not a GGUF model",
            model.name
        )));
    }

    let capabilities = serde_json::from_str::<Vec<String>>(&model.capabilities).unwrap_or_default();
    let supports_chat = capabilities.iter().any(|value| value == "chat");
    let supports_thinking = capabilities.iter().any(|value| value == "thinking");
    let supported = if mode.eq_ignore_ascii_case("thinking") {
        supports_thinking || supports_chat
    } else {
        supports_chat
    };
    if !supported {
        return Err(AppError::ModelUnsupported(format!(
            "{} does not support mobile {mode} generation",
            model.name
        )));
    }
    Ok(())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[allow(clippy::too_many_arguments)]
async fn run_mobile_completion(
    app: &AppHandle,
    state: &State<'_, AppState>,
    native: crate::mobile_inference::MobileInferenceState,
    conversation_id: &str,
    assistant: &Message,
    model_path: std::path::PathBuf,
    model_display: String,
    output_limit: u32,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<Message, AppError> {
    use crate::{
        chat::ChatRepository,
        inference::{StreamChunkEvent, StreamDoneEvent},
        mobile_inference::{generate_android_chat, MobileChatMessage},
    };

    let history = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        ChatRepository::new(&db)
            .list_messages(conversation_id)?
            .into_iter()
            .filter(|message| {
                message.id != assistant.id
                    && (message.role == "system" || message.status == "completed")
            })
            .map(|message| MobileChatMessage {
                role: message.role,
                content: message.content,
            })
            .collect::<Vec<_>>()
    };

    let stream_app = app.clone();
    let stream_conversation_id = conversation_id.to_string();
    let stream_message_id = assistant.id.clone();
    let generation = tokio::task::spawn_blocking(move || {
        generate_android_chat(
            &native,
            model_path,
            model_display,
            history,
            output_limit,
            cancellation,
            move |chunk| {
                stream_app
                    .emit(
                        "inference:chunk",
                        StreamChunkEvent {
                            conversation_id: stream_conversation_id.clone(),
                            message_id: stream_message_id.clone(),
                            chunk: chunk.to_string(),
                        },
                    )
                    .map_err(|error| AppError::StreamFailed(error.to_string()))
            },
        )
    })
    .await
    .map_err(|error| AppError::InferenceFailed(format!("inference worker failed: {error}")))?;

    let generation = match generation {
        Ok(result) => result,
        Err(error) => {
            mark_mobile_generation_failed(state, conversation_id, &assistant.id, app);
            return Err(error);
        }
    };

    let status = if generation.cancelled {
        "cancelled"
    } else {
        "completed"
    };
    let completed = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        let repo = ChatRepository::new(&db);
        if !generation.text.is_empty() {
            repo.append_message_chunk(&assistant.id, &generation.text)?;
        }
        repo.set_message_status(&assistant.id, status)?;
        repo.list_messages(conversation_id)?
            .into_iter()
            .find(|message| message.id == assistant.id)
            .ok_or_else(|| AppError::internal("completed mobile assistant message disappeared"))?
    };

    state.active_generations.finish(conversation_id);
    app.emit(
        "inference:done",
        StreamDoneEvent {
            conversation_id: conversation_id.to_string(),
            message_id: assistant.id.clone(),
            status: status.to_string(),
        },
    )
    .map_err(|error| AppError::StreamFailed(error.to_string()))?;

    Ok(completed)
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn mark_mobile_generation_failed(
    state: &State<'_, AppState>,
    conversation_id: &str,
    assistant_message_id: &str,
    app: &AppHandle,
) {
    use crate::{chat::ChatRepository, inference::StreamDoneEvent};

    if let Ok(db) = state.database.lock() {
        let _ = ChatRepository::new(&db).set_message_status(assistant_message_id, "failed");
    }
    state.active_generations.finish(conversation_id);
    let _ = app.emit(
        "inference:done",
        StreamDoneEvent {
            conversation_id: conversation_id.to_string(),
            message_id: assistant_message_id.to_string(),
            status: "failed".to_string(),
        },
    );
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn validate_mobile_mode(mode: &str) -> Result<(), AppError> {
    if matches!(mode.to_ascii_lowercase().as_str(), "chat" | "thinking") {
        Ok(())
    } else {
        Err(AppError::ModelUnsupported(format!(
            "Mobile on-device inference currently supports chat and thinking modes, not {mode}"
        )))
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn output_limit_for_mode(mode: &str, default_limit: u32) -> u32 {
    if mode.eq_ignore_ascii_case("thinking") {
        default_limit.saturating_mul(2)
    } else {
        default_limit
    }
}
