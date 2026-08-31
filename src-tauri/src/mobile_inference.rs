use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::{app_error::AppError, AppState};

const MOBILE_CONTEXT_TOKENS: u32 = 2048;
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 128;
const MAX_OUTPUT_TOKENS: u32 = 512;

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
    pub model_path: String,
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
        return Err(AppError::ModelUnsupported(
            "native mobile inference is currently available on Android only".to_string(),
        ));
    }

    #[cfg(target_os = "android")]
    {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return Err(AppError::InferenceFailed(
                "prompt cannot be empty".to_string(),
            ));
        }

        // Only allow models below the app-private models directory. The root manager
        // rejects absolute paths and parent traversal before the blocking worker starts.
        let relative = PathBuf::from(relative_model_path.replace('\\', "/"));
        let model_relative = Path::new("models").join(relative);
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

        let output_limit = max_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
            .clamp(1, MAX_OUTPUT_TOKENS);
        let model_display = model_relative.to_string_lossy().replace('\\', "/");

        tokio::task::spawn_blocking(move || {
            generate_android(model_path, model_display, prompt, output_limit)
        })
        .await
        .map_err(|error| AppError::InferenceFailed(format!("inference worker failed: {error}")))?
    }
}

#[cfg(target_os = "android")]
fn generate_android(
    model_path: PathBuf,
    model_display: String,
    prompt: String,
    output_limit: u32,
) -> Result<MobileGenerationResult, AppError> {
    use std::num::NonZeroU32;

    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_backend::LlamaBackend;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::model::{AddBos, LlamaModel};
    use llama_cpp_2::sampling::LlamaSampler;

    let backend = LlamaBackend::init()
        .map_err(|error| AppError::ModelLoadFailed(format!("llama backend init failed: {error}")))?;
    let model = LlamaModel::load_from_file(&backend, &model_path, &LlamaModelParams::default())
        .map_err(|error| AppError::ModelLoadFailed(format!("failed to load GGUF: {error}")))?;

    let threads = std::thread::available_parallelism()
        .map(|count| count.get().min(6) as i32)
        .unwrap_or(4);
    let context_params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(MOBILE_CONTEXT_TOKENS))
        .with_n_threads(threads)
        .with_n_threads_batch(threads);
    let mut context = model
        .new_context(&backend, context_params)
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

    let batch_capacity = prompt_tokens.len().max(1).min(MOBILE_CONTEXT_TOKENS as usize) as i32;
    let mut batch = LlamaBatch::new(batch_capacity, 1);
    let last_prompt_index = prompt_tokens.len().saturating_sub(1) as i32;
    for (position, token) in prompt_tokens.iter().copied().enumerate() {
        batch
            .add(token, position as i32, &[0], position as i32 == last_prompt_index)
            .map_err(|error| AppError::InferenceFailed(format!("batch creation failed: {error}")))?;
    }
    context
        .decode(&mut batch)
        .map_err(|error| AppError::InferenceFailed(format!("prompt decode failed: {error}")))?;

    let mut sampler = LlamaSampler::greedy();
    let mut output = Vec::<u8>::new();
    let mut generated = 0_u32;
    let mut stopped_on_eog = false;
    let mut position = batch.n_tokens();

    while generated < output_limit {
        let token = sampler.sample(&context, batch.n_tokens() - 1);
        sampler.accept(token);
        if model.is_eog_token(token) {
            stopped_on_eog = true;
            break;
        }

        let piece = model
            .token_to_piece_bytes(token, 256, false, None)
            .map_err(|error| AppError::InferenceFailed(format!("token decode failed: {error}")))?;
        output.extend_from_slice(&piece);

        batch.clear();
        batch
            .add(token, position, &[0], true)
            .map_err(|error| AppError::InferenceFailed(format!("generation batch failed: {error}")))?;
        context
            .decode(&mut batch)
            .map_err(|error| AppError::InferenceFailed(format!("generation decode failed: {error}")))?;
        position += 1;
        generated += 1;
    }

    Ok(MobileGenerationResult {
        text: String::from_utf8_lossy(&output).into_owned(),
        prompt_tokens: prompt_tokens.len(),
        generated_tokens: generated,
        stopped_on_eog,
        model_path: model_display,
    })
}
