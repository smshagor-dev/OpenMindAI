use std::{error::Error, fmt, path::Path};

use crate::native_bridge::{GenerationConfig, NativeInferenceEngine};

#[derive(Debug, Clone)]
pub struct InferenceRequest {
    pub prompt: String,
    pub system_prompt: String,
    pub config: GenerationConfig,
}

impl InferenceRequest {
    pub fn new(prompt: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            system_prompt: system_prompt.into(),
            config: GenerationConfig::default(),
        }
    }

    pub fn with_config(mut self, config: GenerationConfig) -> Self {
        self.config = config;
        self
    }
}

#[derive(Debug)]
pub enum InferenceError {
    InvalidConfig(&'static str),
    ModelLoad(String),
    Generation(String),
}

impl fmt::Display for InferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(f, "invalid generation config: {message}"),
            Self::ModelLoad(message) => write!(f, "failed to load native model: {message}"),
            Self::Generation(message) => write!(f, "native generation failed: {message}"),
        }
    }
}

impl Error for InferenceError {}

pub type TokenCallback = Box<dyn FnMut(&str) -> bool + Send + 'static>;

pub trait InferenceBackend {
    fn generate(
        &mut self,
        request: InferenceRequest,
        on_token: TokenCallback,
    ) -> Result<(), InferenceError>;

    fn clear_context(&mut self);
}

pub struct NativeBackend {
    engine: NativeInferenceEngine,
}

impl NativeBackend {
    pub fn load(model_path: &Path, n_gpu_layers: i32) -> Result<Self, InferenceError> {
        let model_path = model_path
            .to_str()
            .ok_or_else(|| InferenceError::ModelLoad("model path is not valid UTF-8".into()))?;
        let engine = NativeInferenceEngine::load(model_path, n_gpu_layers)
            .map_err(|error| InferenceError::ModelLoad(error.to_string()))?;
        Ok(Self { engine })
    }
}

impl InferenceBackend for NativeBackend {
    fn generate(
        &mut self,
        request: InferenceRequest,
        on_token: TokenCallback,
    ) -> Result<(), InferenceError> {
        request
            .config
            .validate()
            .map_err(InferenceError::InvalidConfig)?;
        let config = request.config.normalized();
        self.engine
            .generate_boxed(
                &request.prompt,
                &request.system_prompt,
                config,
                on_token,
            )
            .map_err(|error| InferenceError::Generation(error.to_string()))
    }

    fn clear_context(&mut self) {
        self.engine.clear_kv_cache();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_defaults_to_valid_generation_config() {
        let request = InferenceRequest::new("hello", "be concise");
        assert!(request.config.validate().is_ok());
    }

    #[test]
    fn invalid_config_is_rejected_before_native_generation() {
        let config = GenerationConfig {
            max_tokens: 0,
            ..GenerationConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err("max_tokens must be greater than 0")
        ));
    }
}