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
use tauri::State;

use crate::{
    app_error::AppError, model_registry::ModelRegistry, AppState,
};

const MOBILE_CONTEXT_TOKENS: u32 = 2048;
const MOBILE_MAX_OUTPUT_TOKENS: u32 = 256;
const DEFAULT_PROBE_OUTPUT_TOKENS: u32 = 64;

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

        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| AppError::ModelLoadFailed("native llama backend unavailable".to_string()))?;
        let params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(backend, model_path, &params)
            .map_err(|error| AppError::ModelLoadFailed(error.to_string()))?;
        self.loaded_model = Some(LoadedModel {
            path: model_path.to_path_buf(),
            model,
        });
        Ok(())
    }

    fn generate_chat(
        &mut self,
        model_path: &Path,
        prompt: &str,
        max_tokens: u32,
    ) -> Result<NativeInferenceProbeResult, AppError> {
        self.ensure_model(model_path)?;
        let started = Instant::now();
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| AppError::ModelLoadFailed("native llama backend unavailable".to_string()))?;
        let model = &self
            .loaded_model
            .as_ref()
            .ok_or_else(|| AppError::ModelLoadFailed("native model is not loaded".to_string()))?
            .model;

        let chat_template = model
            .chat_template(None)
            .map_err(|error| AppError::ModelUnsupported(format!("model chat template unavailable: {error}")))?;
        let chat = [LlamaChatMessage::new(
            "user".to_string(),
            prompt.to_string(),
        )
        .map_err(|error| AppError::InferenceFailed(error.to_string()))?];
        let rendered_prompt = model
            .apply_chat_template(&chat_template, &chat, true)
            .map_err(|error| AppError::InferenceFailed(format!("failed to apply model chat template: {error}")))?;
        let prompt_tokens = model
            .str_to_token(&rendered_prompt, AddBos::Always)
            .map_err(|error| AppError::InferenceFailed(format!("failed to tokenize prompt: {error}")))?;

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
        let mut context = model
            .new_context(backend, context_params)
            .map_err(|error| AppError::ModelLoadFailed(format!("failed to create native llama context: {error}")))?;

        let mut batch = LlamaBatch::new(prompt_tokens.len().max(1), 1);
        let last_prompt_index = prompt_tokens.len().saturating_sub(1) as i32;
        for (position, token) in (0_i32..).zip(prompt_tokens.iter().copied()) {
            batch
                .add(token, position, &[0], position == last_prompt_index)
                .map_err(|error| AppError::InferenceFailed(format!("failed to build native prompt batch: {error}")))?;
        }
        context
            .decode(&mut batch)
            .map_err(|error| AppError::InferenceFailed(format!("native prompt decode failed: {error}")))?;

        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_k(20),
            LlamaSampler::top_p(0.95, 1),
            LlamaSampler::temp(0.6),
            LlamaSampler::dist(0x4f4d_4149),
        ]);
        let mut decoder = UTF_8.new_decoder();
        let mut output = String::new();
        let mut generated_tokens = 0_u32;
        let mut position = i32::try_from(prompt_tokens.len())
            .map_err(|_| AppError::ContextOverflow("prompt token count exceeds native position range".to_string()))?;

        for _ in 0..max_tokens {
            let token = sampler.sample(&context, batch.n_tokens() - 1);
            sampler.accept(token);
            if model.is_eog_token(token) {
                break;
            }

            let piece = model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|error| AppError::InferenceFailed(format!("failed to decode native token: {error}")))?;
            output.push_str(&piece);
            generated_tokens += 1;

            batch.clear();
            batch
                .add(token, position, &[0], true)
                .map_err(|error| AppError::InferenceFailed(format!("failed to build native decode batch: {error}")))?;
            context
                .decode(&mut batch)
                .map_err(|error| AppError::InferenceFailed(format!("native token decode failed: {error}")))?;
            position += 1;
        }

        Ok(NativeInferenceProbeResult {
            model_id: String::new(),
            output,
            prompt_tokens: prompt_tokens.len(),
            generated_tokens,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }
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

    let mut result = tokio::task::spawn_blocking(move || {
        let mut engine = engine
            .lock()
            .map_err(|_| AppError::internal("mobile native inference lock poisoned"))?;
        engine.generate_chat(&model_path, &prompt, max_tokens)
    })
    .await
    .map_err(|error| AppError::InferenceFailed(format!("native inference task failed: {error}")))??;

    result.model_id = requested_model_id;
    Ok(result)
}
