mod bridge;

use std::sync::mpsc::SyncSender;

use bridge::{ffi, TokenSink};
use cxx::UniquePtr;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub base_context_tokens: u32,
    /// -1 means "offload as many layers as the compiled backend supports".
    /// Use 0 to force CPU execution.
    pub gpu_layers: i32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            base_context_tokens: 4096,
            gpu_layers: -1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GenerateConfig {
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,
}

impl Default for GenerateConfig {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 1024,
        }
    }
}

#[derive(Debug, Error)]
pub enum NativeInferenceError {
    #[error("model path cannot be empty")]
    EmptyModelPath,
    #[error("prompt cannot be empty")]
    EmptyPrompt,
    #[error("temperature must be finite and between 0 and 5")]
    InvalidTemperature,
    #[error("top_p must be finite and in the range (0, 1]")]
    InvalidTopP,
    #[error("max_tokens must be between 1 and 65536")]
    InvalidMaxTokens,
    #[error("native llama.cpp bridge error: {0}")]
    Bridge(String),
}

pub struct NativeLlamaEngine {
    inner: UniquePtr<ffi::InferenceEngine>,
}

impl NativeLlamaEngine {
    pub fn load(model_path: &str, config: EngineConfig) -> Result<Self, NativeInferenceError> {
        if model_path.trim().is_empty() {
            return Err(NativeInferenceError::EmptyModelPath);
        }

        let inner = ffi::load_model(
            model_path,
            config.base_context_tokens.max(512),
            config.gpu_layers,
        )
        .map_err(|error| NativeInferenceError::Bridge(error.what().to_owned()))?;

        if inner.is_null() {
            return Err(NativeInferenceError::Bridge(
                "llama.cpp returned a null inference engine".to_string(),
            ));
        }
        Ok(Self { inner })
    }

    /// Runs one generation synchronously on the current worker thread.
    ///
    /// `tx` should be connected to a concurrently-drained bounded channel.
    /// Each valid UTF-8 chunk is sent as soon as llama.cpp emits the underlying
    /// token bytes, so callers can forward the chunks directly to Tauri events,
    /// WebSockets, SSE, or a Node.js ThreadsafeFunction.
    pub fn generate_to_sender(
        &mut self,
        prompt: &str,
        system_prompt: &str,
        config: GenerateConfig,
        tx: SyncSender<String>,
    ) -> Result<(), NativeInferenceError> {
        validate_generation(prompt, &config)?;
        let sink = TokenSink::new(tx);

        let result = ffi::generate_stream(
            self.inner.pin_mut(),
            prompt,
            system_prompt,
            config.temperature,
            config.top_p,
            config.max_tokens,
            &sink,
            bridge::push_token,
        )
        .map_err(|error| NativeInferenceError::Bridge(error.what().to_owned()));

        sink.flush();
        result
    }
}

fn validate_generation(prompt: &str, config: &GenerateConfig) -> Result<(), NativeInferenceError> {
    if prompt.trim().is_empty() {
        return Err(NativeInferenceError::EmptyPrompt);
    }
    if !config.temperature.is_finite() || !(0.0..=5.0).contains(&config.temperature) {
        return Err(NativeInferenceError::InvalidTemperature);
    }
    if !config.top_p.is_finite() || config.top_p <= 0.0 || config.top_p > 1.0 {
        return Err(NativeInferenceError::InvalidTopP);
    }
    if config.max_tokens == 0 || config.max_tokens > 65_536 {
        return Err(NativeInferenceError::InvalidMaxTokens);
    }
    Ok(())
}
