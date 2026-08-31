use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use encoding_rs::UTF_8;
use llama_cpp_2::{
    context::params::LlamaContextParams,
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{params::LlamaModelParams, AddBos, LlamaChatMessage, LlamaModel},
    sampling::LlamaSampler,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use crate::{
    app_error::AppError,
    chat::{ChatRepository, Message},
    inference::{StreamChunkEvent, StreamDoneEvent, StreamStartedEvent},
    model_registry::{ModelRecord, ModelRegistry},
    AppState,
};

const MOBILE_CONTEXT_TOKENS: u32 = 2048;
const MOBILE_MAX_OUTPUT_TOKENS: u32 = 256;
const DEFAULT_PROBE_OUTPUT_TOKENS: u32 = 64;
const MOBILE_HISTORY_TURNS: usize = 8;
const MOBILE_NANO_PATH_HINT: &str = "qwen3-0.6b";
const MOBILE_SWIFT_PATH_HINT: &str = "qwen3-1.7b";
const MOBILE_SWIFT_RAM_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Default)]
struct NativeEngine {
    backend: Option<LlamaBackend>,
    loaded_model: Option<LoadedModel>,
}

struct LoadedModel {
    path: PathBuf,
    model: LlamaModel,
}

#[derive(Clone, Default)]
pub(crate) struct MobileInferenceState {
    engine: Arc<Mutex<NativeEngine>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeInferenceProbeResult {
    model_id: String,
    output: String,
    prompt_tokens: usize,
    generated_tokens: u32,
    elapsed_ms: u128,
}

#[derive(Debug)]
struct NativeGenerationResult {
    output: String,
    prompt_tokens: usize,
    generated_tokens: u32,
    elapsed_ms: u128,
}

impl NativeEngine {
    fn ensure_model(&mut self, model_path: &Path) -> Result<(), AppError> {
        if self.backend.is_none() {
            self.backend = Some(
                LlamaBackend::init()
                    .map_err(|error| AppError::ModelLoadFailed(error.to_string()))?,
            );
        }

        let already_loaded = self
            .loaded_model
            .as_ref()
            .is_some_and(|loaded| loaded.path == model_path);
        if already_loaded {
            return Ok(());
        }

        let backend = self.backend.as_ref().ok_or_else(|| {
            AppError::ModelLoadFailed("native llama backend unavailable".to_string())
        })?;
        let params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(backend, model_path, &params)
            .map_err(|error| AppError::ModelLoadFailed(error.to_string()))?;
        self.loaded_model = Some(LoadedModel {
            path: model_path.to_path_buf(),
            model,
        });
        Ok(())
    }

    fn generate_messages<F>(
        &mut self,
        model_path: &Path,
        messages: &[(String, String)],
        max_tokens: u32,
        cancellation: Option<&CancellationToken>,
        mut on_piece: F,
    ) -> Result<NativeGenerationResult, AppError>
    where
        F: FnMut(&str) -> Result<(), AppError>,
    {
        self.ensure_model(model_path)?;
        let started = Instant::now();
        let backend = self.backend.as_ref().ok_or_else(|| {
            AppError::ModelLoadFailed("native llama backend unavailable".to_string())
        })?;
        let model = &self
            .loaded_model
            .as_ref()
            .ok_or_else(|| AppError::ModelLoadFailed("native model is not loaded".to_string()))?
            .model;

        let chat_template = model.chat_template(None).map_err(|error| {
            AppError::ModelUnsupported(format!("model chat template unavailable: {error}"))
        })?;
        let chat = messages
            .iter()
            .map(|(role, content)| {
                LlamaChatMessage::new(role.clone(), content.clone())
                    .map_err(|error| AppError::InferenceFailed(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let rendered_prompt = model
            .apply_chat_template(&chat_template, &chat, true)
            .map_err(|error| {
                AppError::InferenceFailed(format!(
                    "failed to apply model chat template: {error}"
                ))
            })?;
        let prompt_tokens = model
            .str_to_token(&rendered_prompt, AddBos::Always)
            .map_err(|error| {
                AppError::InferenceFailed(format!("failed to tokenize prompt: {error}"))
            })?;

        if prompt_tokens.is_empty() {
            return Err(AppError::InferenceFailed(
                "native tokenizer returned an empty prompt".to_string(),
            ));
        }

        let max_tokens = max_tokens.clamp(1, MOBILE_MAX_OUTPUT_TOKENS);
        let required_tokens = prompt_tokens.len().saturating_add(max_tokens as usize);
        if required_tokens > MOBILE_CONTEXT_TOKENS as usize {
            return Err(AppError::ContextOverflow(format!(
                "mobile native context requires {required_tokens} tokens but the current safety limit is {MOBILE_CONTEXT_TOKENS}"
            )));
        }

        let context_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(MOBILE_CONTEXT_TOKENS))
            .with_n_batch(MOBILE_CONTEXT_TOKENS);
        let mut context = model.new_context(backend, context_params).map_err(|error| {
            AppError::ModelLoadFailed(format!("failed to create native llama context: {error}"))
        })?;

        let mut batch = LlamaBatch::new(prompt_tokens.len().max(1), 1);
        let last_prompt_index = prompt_tokens.len().saturating_sub(1) as i32;
        for (position, token) in (0_i32..).zip(prompt_tokens.iter().copied()) {
            batch
                .add(token, position, &[0], position == last_prompt_index)
                .map_err(|error| {
                    AppError::InferenceFailed(format!(
                        "failed to build native prompt batch: {error}"
                    ))
                })?;
        }
        context.decode(&mut batch).map_err(|error| {
            AppError::InferenceFailed(format!("native prompt decode failed: {error}"))
        })?;

        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_k(20),
            LlamaSampler::top_p(0.95, 1),
            LlamaSampler::temp(0.6),
            LlamaSampler::dist(0x4f4d_4149),
        ]);
        let mut decoder = UTF_8.new_decoder();
        let mut output = String::new();
        let mut generated_tokens = 0_u32;
        let mut position = i32::try_from(prompt_tokens.len()).map_err(|_| {
            AppError::ContextOverflow("prompt token count exceeds native position range".to_string())
        })?;

        for _ in 0..max_tokens {
            if cancellation.is_some_and(CancellationToken::is_cancelled) {
                return Err(AppError::InferenceCancelled(
                    "generation cancelled".to_string(),
                ));
            }

            let token = sampler.sample(&context, batch.n_tokens() - 1);
            sampler.accept(token);
            if model.is_eog_token(token) {
                break;
            }

            let piece = model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|error| {
                    AppError::InferenceFailed(format!("failed to decode native token: {error}"))
                })?;
            if !piece.is_empty() {
                output.push_str(&piece);
                on_piece(&piece)?;
            }
            generated_tokens += 1;

            batch.clear();
            batch.add(token, position, &[0], true).map_err(|error| {
                AppError::InferenceFailed(format!("failed to build native decode batch: {error}"))
            })?;
            context.decode(&mut batch).map_err(|error| {
                AppError::InferenceFailed(format!("native token decode failed: {error}"))
            })?;
            position += 1;
        }

        Ok(NativeGenerationResult {
            output,
            prompt_tokens: prompt_tokens.len(),
            generated_tokens,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }
}

fn mobile_model_score(model: &ModelRecord, total_ram: u64) -> u8 {
    let path = model.path.to_ascii_lowercase();
    let prefer_swift = total_ram >= MOBILE_SWIFT_RAM_BYTES;
    if prefer_swift && path.contains(MOBILE_SWIFT_PATH_HINT) {
        4
    } else if path.contains(MOBILE_NANO_PATH_HINT) {
        3
    } else if path.contains(MOBILE_SWIFT_PATH_HINT) {
        2
    } else if model.capabilities.contains("chat") {
        1
    } else {
        0
    }
}

fn select_mobile_model(
    state: &State<'_, AppState>,
    conversation_id: &str,
) -> Result<ModelRecord, AppError> {
    let database = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let chats = ChatRepository::new(&database);
    let conversation = chats.find_conversation(conversation_id)?;
    let registry = ModelRegistry::new(&database, &state.root);
    let models = registry.list_models()?;

    if let Some(active_id) = conversation.active_model_id.as_deref() {
        if let Some(active) = models
            .iter()
            .find(|model| model.id == active_id && model.enabled)
        {
            return registry.validate_model(&active.id);
        }
    }

    let selected = models
        .into_iter()
        .filter(|model| model.enabled)
        .max_by_key(|model| mobile_model_score(model, state.hardware.memory.total_bytes))
        .filter(|model| mobile_model_score(model, state.hardware.memory.total_bytes) > 0)
        .ok_or_else(|| {
            AppError::ModelNotFound(
                "No mobile-compatible local chat model is installed. Install OpenMindAI Nano or Swift first."
                    .to_string(),
            )
        })?;
    registry.validate_model(&selected.id)
}

fn mobile_chat_history(
    state: &State<'_, AppState>,
    conversation_id: &str,
) -> Result<Vec<(String, String)>, AppError> {
    let database = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let messages = ChatRepository::new(&database).list_messages(conversation_id)?;
    let systems = messages
        .iter()
        .filter(|message| message.status == "completed" && message.role == "system")
        .map(|message| (message.role.clone(), message.content.clone()));
    let dialogue = messages
        .iter()
        .filter(|message| {
            message.status == "completed"
                && matches!(message.role.as_str(), "user" | "assistant")
        })
        .collect::<Vec<_>>();
    let start = dialogue.len().saturating_sub(MOBILE_HISTORY_TURNS);
    Ok(systems
        .chain(
            dialogue[start..]
                .iter()
                .map(|message| (message.role.clone(), message.content.clone())),
        )
        .collect())
}

fn mark_mobile_generation_status(
    state: &State<'_, AppState>,
    assistant_id: &str,
    status: &str,
    content: Option<&str>,
) {
    if let Ok(database) = state.database.lock() {
        let chats = ChatRepository::new(&database);
        if let Some(content) = content.filter(|value| !value.is_empty()) {
            let _ = chats.append_message_chunk(assistant_id, content);
        }
        let _ = chats.set_message_status(assistant_id, status);
    }
}

fn completed_assistant(
    state: &State<'_, AppState>,
    conversation_id: &str,
    assistant_id: &str,
) -> Result<Message, AppError> {
    let database = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&database)
        .list_messages(conversation_id)?
        .into_iter()
        .find(|message| message.id == assistant_id)
        .ok_or_else(|| AppError::internal("completed mobile assistant message disappeared"))
}

#[tauri::command]
pub(crate) async fn send_mobile_chat_message(
    app: AppHandle,
    conversation_id: String,
    content: String,
    mode: String,
    state: State<'_, AppState>,
    native: State<'_, MobileInferenceState>,
) -> Result<Message, AppError> {
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err(AppError::InferenceFailed(
            "message cannot be empty".to_string(),
        ));
    }
    if !matches!(mode.as_str(), "chat" | "thinking") {
        return Err(AppError::ModelUnsupported(
            "Android native inference currently supports Chat and Thinking text turns."
                .to_string(),
        ));
    }

    let cancellation = state.active_generations.start(&conversation_id)?;
    let prepared = (|| {
        let model = select_mobile_model(&state, &conversation_id)?;
        let (user, assistant) = {
            let database = state
                .database
                .lock()
                .map_err(|_| AppError::internal("database lock poisoned"))?;
            let chats = ChatRepository::new(&database);
            let user = chats.add_message(
                &conversation_id,
                "user",
                &content,
                "completed",
                Some(&model.id),
            )?;
            let assistant = chats.add_message(
                &conversation_id,
                "assistant",
                "",
                "streaming",
                Some(&model.id),
            )?;
            (user, assistant)
        };
        let history = mobile_chat_history(&state, &conversation_id)?;
        Ok::<_, AppError>((model, user, assistant, history))
    })();

    let (model, user, assistant, history) = match prepared {
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
            routed_model_name: model.name.clone(),
            routing_reason: format!("Using Android on-device model: {}", model.name),
        },
    ) {
        mark_mobile_generation_status(&state, &assistant.id, "failed", None);
        state.active_generations.finish(&conversation_id);
        return Err(AppError::StreamFailed(error.to_string()));
    }

    let model_path = state.root.resolve_relative(&model.path)?;
    let assistant_id = assistant.id.clone();
    let emit_app = app.clone();
    let emit_conversation_id = conversation_id.clone();
    let emit_assistant_id = assistant_id.clone();
    let engine = native.engine.clone();
    let cancellation_for_worker = cancellation.clone();

    let generated = tokio::task::spawn_blocking(move || {
        let mut engine = engine
            .lock()
            .map_err(|_| AppError::internal("mobile native inference lock poisoned"))?;
        engine.generate_messages(
            &model_path,
            &history,
            MOBILE_MAX_OUTPUT_TOKENS,
            Some(&cancellation_for_worker),
            |piece| {
                emit_app
                    .emit(
                        "inference:chunk",
                        StreamChunkEvent {
                            conversation_id: emit_conversation_id.clone(),
                            message_id: emit_assistant_id.clone(),
                            chunk: piece.to_string(),
                        },
                    )
                    .map_err(|error| AppError::StreamFailed(error.to_string()))
            },
        )
    })
    .await
    .map_err(|error| AppError::InferenceFailed(format!("native inference task failed: {error}")));

    let result = match generated {
        Ok(Ok(result)) => {
            tracing::info!(
                model = %model.name,
                prompt_tokens = result.prompt_tokens,
                generated_tokens = result.generated_tokens,
                elapsed_ms = result.elapsed_ms,
                "completed Android native inference"
            );
            mark_mobile_generation_status(
                &state,
                &assistant_id,
                "completed",
                Some(&result.output),
            );
            state.active_generations.finish(&conversation_id);
            let _ = app.emit(
                "inference:done",
                StreamDoneEvent {
                    conversation_id: conversation_id.clone(),
                    message_id: assistant_id.clone(),
                    status: "completed".to_string(),
                },
            );
            completed_assistant(&state, &conversation_id, &assistant_id)
        }
        Ok(Err(error)) => {
            let status = if matches!(error, AppError::InferenceCancelled(_)) {
                "cancelled"
            } else {
                "failed"
            };
            mark_mobile_generation_status(&state, &assistant_id, status, None);
            state.active_generations.finish(&conversation_id);
            let _ = app.emit(
                "inference:done",
                StreamDoneEvent {
                    conversation_id: conversation_id.clone(),
                    message_id: assistant_id.clone(),
                    status: status.to_string(),
                },
            );
            Err(error)
        }
        Err(error) => {
            let error = AppError::InferenceFailed(format!("native inference task failed: {error}"));
            mark_mobile_generation_status(&state, &assistant_id, "failed", None);
            state.active_generations.finish(&conversation_id);
            let _ = app.emit(
                "inference:done",
                StreamDoneEvent {
                    conversation_id: conversation_id.clone(),
                    message_id: assistant_id.clone(),
                    status: "failed".to_string(),
                },
            );
            Err(error)
        }
    };

    result
}

#[tauri::command]
pub(crate) async fn mobile_native_inference_probe(
    model_id: String,
    prompt: String,
    max_tokens: Option<u32>,
    state: State<'_, AppState>,
    native: State<'_, MobileInferenceState>,
) -> Result<NativeInferenceProbeResult, AppError> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(AppError::InferenceFailed(
            "native inference prompt cannot be empty".to_string(),
        ));
    }
    if prompt.chars().count() > 16_000 {
        return Err(AppError::ContextOverflow(
            "native inference probe prompt is too large".to_string(),
        ));
    }

    let model_path = {
        let database = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        let model = ModelRegistry::new(&database, &state.root).validate_model(&model_id)?;
        state.root.resolve_relative(&model.path)?
    };
    let requested_model_id = model_id.clone();
    let engine = native.engine.clone();
    let max_tokens = max_tokens.unwrap_or(DEFAULT_PROBE_OUTPUT_TOKENS);

    let result = tokio::task::spawn_blocking(move || {
        let mut engine = engine
            .lock()
            .map_err(|_| AppError::internal("mobile native inference lock poisoned"))?;
        engine.generate_messages(
            &model_path,
            &[("user".to_string(), prompt)],
            max_tokens,
            None,
            |_| Ok(()),
        )
    })
    .await
    .map_err(|error| AppError::InferenceFailed(format!("native inference task failed: {error}")))??;

    Ok(NativeInferenceProbeResult {
        model_id: requested_model_id,
        output: result.output,
        prompt_tokens: result.prompt_tokens,
        generated_tokens: result.generated_tokens,
        elapsed_ms: result.elapsed_ms,
    })
}
