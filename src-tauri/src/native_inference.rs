use std::{error::Error, fmt, path::Path};

use crate::native_bridge::{ChatMessage, GenerationConfig, NativeInferenceEngine};

#[derive(Debug, Clone)]
pub struct InferenceRequest {
    pub messages: Vec<ChatMessage>,
    pub config: GenerationConfig,
}

impl InferenceRequest {
    pub fn new(prompt: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        let system_prompt = system_prompt.into();
        let mut messages = Vec::with_capacity(if system_prompt.is_empty() { 1 } else { 2 });
        if !system_prompt.is_empty() {
            messages.push(ChatMessage::system(system_prompt));
        }
        messages.push(ChatMessage::user(prompt));
        Self {
            messages,
            config: GenerationConfig::default(),
        }
    }

    pub fn from_messages(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            config: GenerationConfig::default(),
        }
    }

    pub fn with_config(mut self, config: GenerationConfig) -> Self {
        self.config = config;
        self
    }

    pub fn validate(&self) -> Result<(), InferenceError> {
        if self.messages.is_empty() {
            return Err(InferenceError::InvalidRequest(
                "chat history must contain at least one message",
            ));
        }
        for message in &self.messages {
            if !matches!(message.role.as_str(), "system" | "user" | "assistant") {
                return Err(InferenceError::InvalidRequest(
                    "chat history contains an unsupported role",
                ));
            }
            if message.content.trim().is_empty() {
                return Err(InferenceError::InvalidRequest(
                    "chat history contains an empty message",
                ));
            }
        }
        self.config
            .validate()
            .map_err(InferenceError::InvalidConfig)
    }
}

#[derive(Debug)]
pub enum InferenceError {
    InvalidConfig(&'static str),
    InvalidRequest(&'static str),
    ModelLoad(String),
    Generation(String),
}

impl fmt::Display for InferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(f, "invalid generation config: {message}"),
            Self::InvalidRequest(message) => write!(f, "invalid inference request: {message}"),
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
        request.validate()?;
        let config = request.config.normalized();
        self.engine
            .generate_messages_boxed(&request.messages, config, on_token)
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
        assert!(request.validate().is_ok());
        assert_eq!(request.messages.len(), 2);
    }

    #[test]
    fn multi_turn_request_preserves_history() {
        let request = InferenceRequest::from_messages(vec![
            ChatMessage::system("be concise"),
            ChatMessage::user("one"),
            ChatMessage::assistant("two"),
            ChatMessage::user("three"),
        ]);
        assert!(request.validate().is_ok());
        assert_eq!(request.messages.len(), 4);
        assert_eq!(request.messages[2].role, "assistant");
    }

    #[test]
    fn invalid_config_is_rejected_before_native_generation() {
        let config = GenerationConfig {
            max_tokens: 0,
            ..GenerationConfig::default()
        };
        let request = InferenceRequest::new("hello", "").with_config(config);
        assert!(matches!(
            request.validate(),
            Err(InferenceError::InvalidConfig(
                "max_tokens must be greater than 0"
            ))
        ));
    }

    #[test]
    fn invalid_role_is_rejected_before_native_generation() {
        let request = InferenceRequest::from_messages(vec![ChatMessage::new("tool", "result")]);
        assert!(matches!(
            request.validate(),
            Err(InferenceError::InvalidRequest(
                "chat history contains an unsupported role"
            ))
        ));
    }
}
