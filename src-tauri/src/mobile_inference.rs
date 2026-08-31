use std::path::Path;
#[cfg(target_os = "android")]
use std::path::{Component, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::{app_error::AppError, AppState};

const MOBILE_CONTEXT_TOKENS: u32 = 2048;
pub(crate) const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 128;
const MAX_OUTPUT_TOKENS: u32 = 512;
const MAX_CHAT_HISTORY_MESSAGES: usize = 48;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileInferenceStatus {
    pub supported: bool,
    pub backend: &'static str,
    pub model_count: usize,
    pub models: Vec<String>,
    pub context_tokens: u32,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileGenerationResult {
    pub text: String,
    pub prompt_tokens: usize,
    pub generated_tokens: u32,
    pub stopped_on_eog: bool,
    pub cancelled: bool,
    /// Path relative to the app-private `models/` directory.
    pub model_path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct MobileChatMessage {
    pub role: String,
    pub content: String,
}

fn collect_gguf_models(root: &Path) -> Vec<String> {
    fn visit(base: &Path, dir: &Path, output: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(base, &path, output);
            } else if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("gguf"))
            {
                if let Ok(relative) = path.strip_prefix(base) {
                    output.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }

    let mut models = Vec::new();
    visit(root, root, &mut models);
    models.sort();
    models
}

#[tauri::command]
pub fn mobile_local_inference_status(
    state: State<'_, AppState>,
) -> Result<MobileInferenceStatus, AppError> {
    let models = collect_gguf_models(&state.root.models_dir());
    Ok(MobileInferenceStatus {
        supported: cfg!(target_os = "android"),
        backend: if cfg!(target_os = "android") {
            "llama.cpp-native-cpu"
        } else {
            "unsupported"
        },
        model_count: models.len(),
        models,
        context_tokens: MOBILE_CONTEXT_TOKENS,
        max_output_tokens: MAX_OUTPUT_TOKENS,
    })
}

#[tauri::command]
pub async fn mobile_generate_text(
    relative_model_path: String,
    prompt: String,
    max_tokens: Option<u32>,
    state: State<'_, AppState>,
) -> Result<MobileGenerationResult, AppError> {
    #[cfg(not(target_os = "android"))]
    {
        let _ = (relative_model_path, prompt, max_tokens, state);
        Err(AppError::ModelUnsupported(
            "native mobile inference is currently available on Android only".to_string(),
        ))
    }

    #[cfg(target_os = "android")]
    {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return Err(AppError::InferenceFailed(
                "prompt cannot be empty".to_string(),
            ));
        }

        let (model_path, model_display) =
            resolve_android_model_path(&state, &relative_model_path)?;
        let output_limit = max_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
            .clamp(1, MAX_OUTPUT_TOKENS);

        tokio::task::spawn_blocking(move || {
            generate_android_prompt(model_path, model_display, prompt, output_limit, None)
        })
        .await
        .map_err(|error| AppError::InferenceFailed(format!("inference worker failed: {error}")))?
    }
}

#[cfg(target_os = "android")]
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

#[cfg(target_os = "android")]
pub(crate) fn generate_android_chat(
    model_path: PathBuf,
    model_display: String,
    messages: Vec<MobileChatMessage>,
    output_limit: u32,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<MobileGenerationResult, AppError> {
    use llama_cpp_2::llama_backend::LlamaBackend;
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::model::LlamaModel;

    let backend = LlamaBackend::init().map_err(|error| {
        AppError::ModelLoadFailed(format!("llama backend init failed: {error}"))
    })?;
    let model = LlamaModel::load_from_file(&backend, &model_path, &LlamaModelParams::default())
        .map_err(|error| AppError::ModelLoadFailed(format!("failed to load GGUF: {error}")))?;
    let prompt = build_android_chat_prompt(&model, messages, output_limit)?;

    run_loaded_generation(
        &backend,
        &model,
        model_display,
        prompt,
        output_limit,
        Some(&cancellation),
    )
}

#[cfg(target_os = "android")]
fn generate_android_prompt(
    model_path: PathBuf,
    model_display: String,
    prompt: String,
    output_limit: u32,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> Result<MobileGenerationResult, AppError> {
    use llama_cpp_2::llama_backend::LlamaBackend;
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::model::LlamaModel;

    let backend = LlamaBackend::init().map_err(|error| {
        AppError::ModelLoadFailed(format!("llama backend init failed: {error}"))
    })?;
    let model = LlamaModel::load_from_file(&backend, &model_path, &LlamaModelParams::default())
        .map_err(|error| AppError::ModelLoadFailed(format!("failed to load GGUF: {error}")))?;

    run_loaded_generation(
        &backend,
        &model,
        model_display,
        prompt,
        output_limit,
        cancellation,
    )
}

#[cfg(target_os = "android")]
fn build_android_chat_prompt(
    model: &llama_cpp_2::model::LlamaModel,
    messages: Vec<MobileChatMessage>,
    output_limit: u32,
) -> Result<String, AppError> {
    use llama_cpp_2::model::{AddBos, LlamaChatMessage};

    let template = model.chat_template(None).map_err(|error| {
        AppError::InferenceFailed(format!(
            "the installed GGUF does not expose a usable chat template: {error}"
        ))
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

    if conversation_messages
        .iter()
        .rposition(|message| message.role == "user")
        .is_none()
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
        let prompt = model
            .apply_chat_template(&template, &llama_messages, true)
            .map_err(|error| {
                AppError::InferenceFailed(format!("chat template application failed: {error}"))
            })?;
        let prompt_tokens = model
            .str_to_token(&prompt, AddBos::Always)
            .map_err(|error| AppError::InferenceFailed(format!("tokenization failed: {error}")))?;

        if prompt_tokens.len() + output_limit as usize < MOBILE_CONTEXT_TOKENS as usize {
            return Ok(prompt);
        }

        let last_user_index = working.iter().rposition(|message| message.role == "user");
        let removable = working.iter().enumerate().find_map(|(index, message)| {
            (message.role != "system" && Some(index) != last_user_index).then_some(index)
        });
        let Some(index) = removable else {
            return Err(AppError::ContextOverflow(format!(
                "the latest mobile chat turn does not fit inside the {} token context window",
                MOBILE_CONTEXT_TOKENS
            )));
        };
        working.remove(index);
    }
}

#[cfg(target_os = "android")]
fn run_loaded_generation(
    backend: &llama_cpp_2::llama_backend::LlamaBackend,
    model: &llama_cpp_2::model::LlamaModel,
    model_display: String,
    prompt: String,
    output_limit: u32,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> Result<MobileGenerationResult, AppError> {
    use std::num::NonZeroU32;

    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::AddBos;
    use llama_cpp_2::sampling::LlamaSampler;

    let threads = std::thread::available_parallelism()
        .map(|count| count.get().min(6) as i32)
        .unwrap_or(4);
    let context_params = LlamaContextParams::default()
        .with_n_ctx(Some(
            NonZeroU32::new(MOBILE_CONTEXT_TOKENS).expect("context must be non-zero"),
        ))
        .with_n_threads(threads)
        .with_n_threads_batch(threads);
    let mut context = model
        .new_context(backend, context_params)
        .map_err(|error| AppError::ModelLoadFailed(format!("failed to create context: {error}")))?;

    let prompt_tokens = model
        .str_to_token(&prompt, AddBos::Always)
        .map_err(|error| AppError::InferenceFailed(format!("tokenization failed: {error}")))?;
    if prompt_tokens.len() + output_limit as usize >= MOBILE_CONTEXT_TOKENS as usize {
        return Err(AppError::ContextOverflow(format!(
            "prompt uses {} tokens; Android context is limited to {} tokens",
            prompt_tokens.len(),
            MOBILE_CONTEXT_TOKENS
        )));
    }

    let batch_capacity = prompt_tokens
        .len()
        .max(1)
        .min(MOBILE_CONTEXT_TOKENS as usize);
    let mut batch = LlamaBatch::new(batch_capacity, 1);
    let last_prompt_index = prompt_tokens.len().saturating_sub(1) as i32;
    for (position, token) in prompt_tokens.iter().copied().enumerate() {
        batch
            .add(
                token,
                position as i32,
                &[0],
                position as i32 == last_prompt_index,
            )
            .map_err(|error| {
                AppError::InferenceFailed(format!("batch creation failed: {error}"))
            })?;
    }
    context
        .decode(&mut batch)
        .map_err(|error| AppError::InferenceFailed(format!("prompt decode failed: {error}")))?;

    let mut sampler = LlamaSampler::greedy();
    let mut output = Vec::<u8>::new();
    let mut generated = 0_u32;
    let mut stopped_on_eog = false;
    let mut cancelled = false;
    let mut position = batch.n_tokens();

    while generated < output_limit {
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
            .token_to_piece_bytes(token, 4096, false, None)
            .map_err(|error| AppError::InferenceFailed(format!("token decode failed: {error}")))?;
        output.extend_from_slice(&piece);

        batch.clear();
        batch.add(token, position, &[0], true).map_err(|error| {
            AppError::InferenceFailed(format!("generation batch failed: {error}"))
        })?;
        context.decode(&mut batch).map_err(|error| {
            AppError::InferenceFailed(format!("generation decode failed: {error}"))
        })?;
        position += 1;
        generated += 1;
    }

    Ok(MobileGenerationResult {
        text: String::from_utf8_lossy(&output).into_owned(),
        prompt_tokens: prompt_tokens.len(),
        generated_tokens: generated,
        stopped_on_eog,
        cancelled,
        model_path: model_display,
    })
}
