use std::{
    num::NonZeroU32,
    path::{Component, Path, PathBuf},
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
use tauri::State;

use crate::{app_error::AppError, model_registry::ModelRegistry, AppState};

const MOBILE_CONTEXT_TOKENS: u32 = 2048;
const MOBILE_MAX_OUTPUT_TOKENS: u32 = 256;
pub(crate) const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 128;
const DEFAULT_PROBE_OUTPUT_TOKENS: u32 = 64;
const MAX_CHAT_HISTORY_MESSAGES: usize = 48;

type StreamCallback<'a> = dyn FnMut(&str) -> Result<(), AppError> + 'a;

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

#[derive(Debug, Clone)]
pub(crate) struct MobileChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub(crate) struct MobileGenerationResult {
    pub text: String,
    pub prompt_tokens: usize,
    pub generated_tokens: u32,
    pub stopped_on_eog: bool,
    pub cancelled: bool,
    pub model_path: String,
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
        self.loaded_model = None;
        let params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(backend, model_path, &params)
            .map_err(|error| AppError::ModelLoadFailed(error.to_string()))?;
        self.loaded_model = Some(LoadedModel {
            path: model_path.to_path_buf(),
            model,
        });
        Ok(())
    }

    fn generate_messages(
        &mut self,
        model_path: &Path,
        model_display: String,
        messages: Vec<MobileChatMessage>,
        max_tokens: u32,
        cancellation: Option<&tokio_util::sync::CancellationToken>,
        mut on_chunk: Option<&mut StreamCallback<'_>>,
    ) -> Result<MobileGenerationResult, AppError> {
        self.ensure_model(model_path)?;
        let backend = self.backend.as_ref().ok_or_else(|| {
            AppError::ModelLoadFailed("native llama backend unavailable".to_string())
        })?;
        let model = &self
            .loaded_model
            .as_ref()
            .ok_or_else(|| AppError::ModelLoadFailed("native model is not loaded".to_string()))?
            .model;

        let max_tokens = max_tokens.clamp(1, MOBILE_MAX_OUTPUT_TOKENS);
        let rendered_prompt = build_chat_prompt(model, messages, max_tokens)?;
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

        let required_tokens = prompt_tokens.len().saturating_add(max_tokens as usize);
        if required_tokens > MOBILE_CONTEXT_TOKENS as usize {
            return Err(AppError::ContextOverflow(format!(
                "mobile native context requires {required_tokens} tokens but the current safety limit is {MOBILE_CONTEXT_TOKENS}"
            )));
        }

        let context_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(MOBILE_CONTEXT_TOKENS))
            .with_n_batch(MOBILE_CONTEXT_TOKENS);
        let mut context = model
            .new_context(backend, context_params)
            .map_err(|error| {
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
        let mut stopped_on_eog = false;
        let mut cancelled = false;
        let mut position = i32::try_from(prompt_tokens.len()).map_err(|_| {
            AppError::ContextOverflow(
                "prompt token count exceeds native position range".to_string(),
            )
        })?;

        for _ in 0..max_tokens {
            if cancellation.is_some_and(tokio_util::sync::CancellationToken::is_cancelled) {
                cancelled = true;
                break;
            }

            let token = sampler.sample(&context, batch.n_tokens() - 1);
            sampler.accept(token);
            if model.is_eog_token(token) {
                stopped_on_eog = true;
                break;
            }

            let piece = model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|error| {
                    AppError::InferenceFailed(format!("failed to decode native token: {error}"))
                })?;
            if !piece.is_empty() {
                output.push_str(&piece);
                if let Some(callback) = on_chunk.as_deref_mut() {
                    callback(&piece)?;
                }
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

        Ok(MobileGenerationResult {
            text: output,
            prompt_tokens: prompt_tokens.len(),
            generated_tokens,
            stopped_on_eog,
            cancelled,
            model_path: model_display,
        })
    }

    fn generate_probe(
        &mut self,
        model_path: &Path,
        model_id: String,
        prompt: String,
        max_tokens: u32,
    ) -> Result<NativeInferenceProbeResult, AppError> {
        let started = Instant::now();
        let result = self.generate_messages(
            model_path,
            model_id.clone(),
            vec![MobileChatMessage {
                role: "user".to_string(),
                content: prompt,
            }],
            max_tokens,
            None,
            None,
        )?;

        Ok(NativeInferenceProbeResult {
            model_id,
            output: result.text,
            prompt_tokens: result.prompt_tokens,
            generated_tokens: result.generated_tokens,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }
}

fn build_chat_prompt(
    model: &LlamaModel,
    messages: Vec<MobileChatMessage>,
    output_limit: u32,
) -> Result<String, AppError> {
    let template = model.chat_template(None).map_err(|error| {
        AppError::ModelUnsupported(format!("model chat template unavailable: {error}"))
    })?;

    let mut system_messages = Vec::new();
    let mut conversation_messages = Vec::new();
    for message in messages {
        let role = message.role.trim().to_ascii_lowercase();
        let content = message.content.trim().to_string();
        if content.is_empty() || !matches!(role.as_str(), "system" | "user" | "assistant") {
            continue;
        }
        let normalized = MobileChatMessage { role, content };
        if normalized.role == "system" {
            system_messages.push(normalized);
        } else {
            conversation_messages.push(normalized);
        }
    }

    if !conversation_messages
        .iter()
        .any(|message| message.role == "user")
    {
        return Err(AppError::InferenceFailed(
            "mobile chat history has no user message to answer".to_string(),
        ));
    }

    if conversation_messages.len() > MAX_CHAT_HISTORY_MESSAGES {
        let keep_from = conversation_messages.len() - MAX_CHAT_HISTORY_MESSAGES;
        conversation_messages.drain(..keep_from);
    }

    let mut working = system_messages;
    working.extend(conversation_messages);

    loop {
        let llama_messages = working
            .iter()
            .map(|message| LlamaChatMessage::new(message.role.clone(), message.content.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                AppError::InferenceFailed(format!("chat message encoding failed: {error}"))
            })?;
        let rendered_prompt = model
            .apply_chat_template(&template, &llama_messages, true)
            .map_err(|error| {
                AppError::InferenceFailed(format!("failed to apply model chat template: {error}"))
            })?;
        let prompt_tokens = model
            .str_to_token(&rendered_prompt, AddBos::Always)
            .map_err(|error| {
                AppError::InferenceFailed(format!("failed to tokenize prompt: {error}"))
            })?;

        if prompt_tokens.len().saturating_add(output_limit as usize)
            <= MOBILE_CONTEXT_TOKENS as usize
        {
            return Ok(rendered_prompt);
        }

        let last_user_index = working.iter().rposition(|message| message.role == "user");
        let removable = working.iter().enumerate().find_map(|(index, message)| {
            (message.role != "system" && Some(index) != last_user_index).then_some(index)
        });
        let Some(index) = removable else {
            return Err(AppError::ContextOverflow(format!(
                "the latest mobile chat turn does not fit inside the {MOBILE_CONTEXT_TOKENS} token context window"
            )));
        };
        working.remove(index);
    }
}

pub(crate) fn resolve_android_model_path(
    state: &AppState,
    relative_model_path: &str,
) -> Result<(PathBuf, String), AppError> {
    let normalized = relative_model_path.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err(AppError::ModelNotFound(
            "no Android model path was provided".to_string(),
        ));
    }

    let relative = PathBuf::from(&normalized);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(AppError::ModelUnsupported(
            "Android model paths must stay inside the app-private models directory".to_string(),
        ));
    }

    let model_relative = Path::new("models").join(&relative);
    let model_path = state.root.resolve_relative(&model_relative)?;
    if !model_path.is_file() {
        return Err(AppError::ModelNotFound(model_path.display().to_string()));
    }
    if !model_path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("gguf"))
    {
        return Err(AppError::ModelUnsupported(
            "Android native inference requires a GGUF model".to_string(),
        ));
    }

    Ok((model_path, normalized))
}

pub(crate) fn generate_android_chat(
    native: &MobileInferenceState,
    model_path: PathBuf,
    model_display: String,
    messages: Vec<MobileChatMessage>,
    output_limit: u32,
    cancellation: tokio_util::sync::CancellationToken,
    mut on_chunk: impl FnMut(&str) -> Result<(), AppError>,
) -> Result<MobileGenerationResult, AppError> {
    let mut engine = native
        .engine
        .lock()
        .map_err(|_| AppError::internal("mobile native inference lock poisoned"))?;
    engine.generate_messages(
        &model_path,
        model_display,
        messages,
        output_limit,
        Some(&cancellation),
        Some(&mut on_chunk),
    )
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

    tokio::task::spawn_blocking(move || {
        let mut engine = engine
            .lock()
            .map_err(|_| AppError::internal("mobile native inference lock poisoned"))?;
        engine.generate_probe(&model_path, requested_model_id, prompt, max_tokens)
    })
    .await
    .map_err(|error| AppError::InferenceFailed(format!("native inference task failed: {error}")))?
}
