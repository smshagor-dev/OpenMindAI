#[cxx::bridge(namespace = "openmind")]
mod ffi {
    #[derive(Debug, Clone)]
    struct ChatMessage {
        role: String,
        content: String,
    }

    #[derive(Debug, Clone, Copy)]
    struct GenerationConfig {
        temperature: f32,
        top_p: f32,
        max_tokens: u32,
        n_ctx: u32,
        n_batch: u32,
        n_threads: i32,
        kv_cache_limit_bytes: u64,
        timeout_ms: u32,
    }

    extern "Rust" {
        type TokenSink;
        fn on_token(sink: &mut TokenSink, token: &[u8]) -> bool;
    }

    unsafe extern "C++" {
        include!("openmind/native/inference.h");

        type InferenceEngine;

        fn create_engine(model_path: &str, n_gpu_layers: i32)
            -> Result<UniquePtr<InferenceEngine>>;

        fn generate_messages(
            self: Pin<&mut InferenceEngine>,
            messages: &[ChatMessage],
            config: &GenerationConfig,
            sink: &mut TokenSink,
        ) -> Result<()>;

        fn clear_kv_cache(self: Pin<&mut InferenceEngine>);
    }
}

pub use ffi::{ChatMessage, GenerationConfig};

impl ChatMessage {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new("system", content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new("user", content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new("assistant", content)
    }
}

impl Default for GenerationConfig {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 512,
            n_ctx: 8_192,
            n_batch: 512,
            n_threads: 1,
            kv_cache_limit_bytes: 2 * 1024 * 1024 * 1024,
            timeout_ms: 120_000,
        }
    }
}

impl GenerationConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.temperature.is_finite() || self.temperature < 0.0 {
            return Err("temperature must be finite and >= 0");
        }
        if !self.top_p.is_finite() || self.top_p <= 0.0 || self.top_p > 1.0 {
            return Err("top_p must be finite and in (0, 1]");
        }
        if self.max_tokens == 0 {
            return Err("max_tokens must be greater than 0");
        }
        if self.n_ctx > 32_768
            || self.max_tokens > 8_192
            || self.n_batch > 2_048
            || self.n_threads > 256
        {
            return Err("generation resource limit exceeded");
        }
        if !(1..=3_600_000).contains(&self.timeout_ms) {
            return Err("timeout_ms must be 1..3600000");
        }
        if !(16 * 1024 * 1024..=4 * 1024 * 1024 * 1024).contains(&self.kv_cache_limit_bytes) {
            return Err("KV budget must be between 16 MiB and 4 GiB");
        }
        if self.n_ctx != 0 && self.n_ctx < 512 {
            return Err("n_ctx must be 0 (automatic) or at least 512");
        }
        if self.n_threads <= 0 {
            return Err("n_threads must be greater than 0");
        }
        Ok(())
    }

    pub fn normalized(mut self) -> Self {
        if self.n_ctx == 0 {
            self.n_ctx = 8_192;
        }
        if self.n_batch == 0 {
            self.n_batch = 512;
        }
        self
    }
}

pub struct TokenSink {
    callback: Box<dyn FnMut(&str) -> bool + Send>,
    pending: Vec<u8>,
    stopped: bool,
}

impl TokenSink {
    pub fn new<F>(callback: F) -> Self
    where
        F: FnMut(&str) -> bool + Send + 'static,
    {
        Self::from_boxed(Box::new(callback))
    }

    pub fn from_boxed(callback: Box<dyn FnMut(&str) -> bool + Send>) -> Self {
        Self {
            callback,
            pending: Vec::new(),
            stopped: false,
        }
    }

    fn emit(&mut self, text: &str) -> bool {
        if self.stopped {
            return false;
        }
        self.stopped = !(self.callback)(text);
        !self.stopped
    }

    fn finish(&mut self) {
        // Only a normal end may replace a truncated final code point. A stopped
        // stream must never call the consumer again or invent cancelled output.
        if !self.stopped && !self.pending.is_empty() {
            let remaining = String::from_utf8_lossy(&self.pending).into_owned();
            self.pending.clear();
            self.emit(&remaining);
        }
    }
}

fn on_token(sink: &mut TokenSink, token: &[u8]) -> bool {
    if sink.stopped {
        return false;
    }
    // llama token pieces may end in the middle of a UTF-8 code point. Never
    // construct a Rust str on the C++ side before the bytes have been validated.
    sink.pending.extend_from_slice(token);
    let mut text = String::new();
    let mut consumed = 0;
    while consumed < sink.pending.len() {
        match std::str::from_utf8(&sink.pending[consumed..]) {
            Ok(valid) => {
                text.push_str(valid);
                consumed = sink.pending.len();
            }
            Err(error) => {
                let valid_end = consumed + error.valid_up_to();
                text.push_str(std::str::from_utf8(&sink.pending[consumed..valid_end]).unwrap());
                consumed = valid_end;
                match error.error_len() {
                    Some(length) => {
                        text.push('\u{fffd}');
                        consumed += length;
                    }
                    None => break,
                }
            }
        }
    }
    sink.pending.drain(..consumed);
    // Empty text is also a cooperative cancellation poll during prompt decode
    // and when a token contains only the beginning of a multi-byte character.
    sink.emit(&text)
}

pub struct NativeInferenceEngine {
    inner: cxx::UniquePtr<ffi::InferenceEngine>,
}

impl NativeInferenceEngine {
    pub fn load(model_path: &str, n_gpu_layers: i32) -> Result<Self, cxx::Exception> {
        let inner = ffi::create_engine(model_path, n_gpu_layers)?;
        Ok(Self { inner })
    }

    pub fn generate<F>(
        &mut self,
        prompt: &str,
        system_prompt: &str,
        config: GenerationConfig,
        callback: F,
    ) -> Result<(), cxx::Exception>
    where
        F: FnMut(&str) -> bool + Send + 'static,
    {
        self.generate_boxed(prompt, system_prompt, config, Box::new(callback))
    }

    pub fn generate_boxed(
        &mut self,
        prompt: &str,
        system_prompt: &str,
        config: GenerationConfig,
        callback: Box<dyn FnMut(&str) -> bool + Send>,
    ) -> Result<(), cxx::Exception> {
        let mut messages = Vec::with_capacity(if system_prompt.is_empty() { 1 } else { 2 });
        if !system_prompt.is_empty() {
            messages.push(ChatMessage::system(system_prompt));
        }
        messages.push(ChatMessage::user(prompt));
        self.generate_messages_boxed(&messages, config, callback)
    }

    pub fn generate_messages<F>(
        &mut self,
        messages: &[ChatMessage],
        config: GenerationConfig,
        callback: F,
    ) -> Result<(), cxx::Exception>
    where
        F: FnMut(&str) -> bool + Send + 'static,
    {
        self.generate_messages_boxed(messages, config, Box::new(callback))
    }

    pub fn generate_messages_boxed(
        &mut self,
        messages: &[ChatMessage],
        config: GenerationConfig,
        callback: Box<dyn FnMut(&str) -> bool + Send>,
    ) -> Result<(), cxx::Exception> {
        let mut sink = TokenSink::from_boxed(callback);
        let result = self
            .inner
            .pin_mut()
            .generate_messages(messages, &config, &mut sink);
        if result.is_ok() {
            sink.finish();
        }
        result
    }

    pub fn clear_kv_cache(&mut self) {
        self.inner.pin_mut().clear_kv_cache();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn recording_sink() -> (TokenSink, Arc<Mutex<String>>) {
        let output = Arc::new(Mutex::new(String::new()));
        let captured = Arc::clone(&output);
        let sink = TokenSink::new(move |text| {
            captured.lock().unwrap().push_str(text);
            true
        });
        (sink, output)
    }

    #[test]
    fn resource_limits_reject_unbounded_configuration() {
        for config in [
            GenerationConfig {
                n_ctx: 32769,
                ..GenerationConfig::default()
            },
            GenerationConfig {
                max_tokens: 8193,
                ..GenerationConfig::default()
            },
            GenerationConfig {
                timeout_ms: 0,
                ..GenerationConfig::default()
            },
            GenerationConfig {
                kv_cache_limit_bytes: 1,
                ..GenerationConfig::default()
            },
        ] {
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn unicode_survives_every_token_boundary() {
        let expected = "বাংলা: ভালো আছি 🦀🙂 café 中文";
        for split in 0..=expected.len() {
            let (mut sink, output) = recording_sink();
            assert!(on_token(&mut sink, &expected.as_bytes()[..split]));
            assert!(on_token(&mut sink, &expected.as_bytes()[split..]));
            sink.finish();
            assert_eq!(*output.lock().unwrap(), expected, "split {split}");
        }
        let (mut sink, output) = recording_sink();
        for byte in expected.as_bytes() {
            assert!(on_token(&mut sink, &[*byte]));
            assert!(sink.pending.len() <= 3);
        }
        sink.finish();
        assert_eq!(*output.lock().unwrap(), expected);
    }

    #[test]
    fn malformed_bytes_are_replaced_without_losing_following_text() {
        let bytes = b"ok\xff\xe2\x82next\xf0\x9f\x99\x82";
        for split in 0..=bytes.len() {
            let (mut sink, output) = recording_sink();
            on_token(&mut sink, &bytes[..split]);
            on_token(&mut sink, &bytes[split..]);
            sink.finish();
            assert_eq!(*output.lock().unwrap(), String::from_utf8_lossy(bytes));
        }
    }

    #[test]
    fn incomplete_final_character_is_flushed_once() {
        let (mut sink, output) = recording_sink();
        on_token(&mut sink, b"ok\xe2\x82");
        assert_eq!(*output.lock().unwrap(), "ok");
        sink.finish();
        sink.finish();
        assert_eq!(*output.lock().unwrap(), "ok\u{fffd}");
    }

    #[test]
    fn cancellation_poll_does_not_flush_incomplete_character() {
        let mut sink = TokenSink::new(|_| false);
        assert!(!on_token(&mut sink, b"\xe2"));
        sink.finish();
        assert!(sink.stopped);
        assert!(!on_token(&mut sink, b"\x82\xac"));
    }

    #[test]
    fn token_sink_can_cancel_streaming() {
        let mut seen = Vec::new();
        let mut sink = TokenSink::new(move |token| {
            seen.push(token.to_owned());
            token != "stop"
        });

        assert!(on_token(&mut sink, b"one"));
        assert!(!on_token(&mut sink, b"stop"));
        assert!(!on_token(&mut sink, b"ignored"));
    }

    #[test]
    fn chat_message_helpers_preserve_roles() {
        let messages = [
            ChatMessage::system("rules"),
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi"),
        ];
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[2].role, "assistant");
    }

    #[test]
    fn generation_config_validates_boundary_values() {
        let config = GenerationConfig {
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 256,
            n_ctx: 8_192,
            n_batch: 512,
            n_threads: 8,
            ..GenerationConfig::default()
        };
        assert_eq!(config.validate(), Ok(()));

        let invalid = GenerationConfig {
            top_p: 0.0,
            ..config
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn generation_config_normalizes_automatic_sizes() {
        let config = GenerationConfig {
            n_ctx: 0,
            n_batch: 0,
            ..GenerationConfig::default()
        }
        .normalized();
        assert_eq!(config.n_ctx, 8_192);
        assert_eq!(config.n_batch, 512);
    }
}
