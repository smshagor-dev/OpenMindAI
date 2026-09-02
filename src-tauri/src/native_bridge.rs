use std::pin::Pin;

#[cxx::bridge(namespace = "openmind")]
mod ffi {
    #[derive(Debug, Clone, Copy)]
    struct GenerationConfig {
        temperature: f32,
        top_p: f32,
        max_tokens: u32,
        n_ctx: u32,
        n_batch: u32,
        n_threads: i32,
        n_gpu_layers: i32,
    }

    extern "Rust" {
        type TokenSink;
        fn on_token(sink: &mut TokenSink, token: &str) -> bool;
    }

    unsafe extern "C++" {
        include!("openmind/native/inference.h");

        type InferenceEngine;

        fn create_engine(model_path: &str, n_gpu_layers: i32)
            -> Result<UniquePtr<InferenceEngine>>;

        fn generate(
            self: Pin<&mut InferenceEngine>,
            prompt: &str,
            system_prompt: &str,
            config: &GenerationConfig,
            sink: &mut TokenSink,
        ) -> Result<()>;

        fn clear_kv_cache(self: Pin<&mut InferenceEngine>);
    }
}

pub use ffi::GenerationConfig;

pub struct TokenSink {
    callback: Box<dyn FnMut(&str) -> bool + Send>,
}

impl TokenSink {
    pub fn new<F>(callback: F) -> Self
    where
        F: FnMut(&str) -> bool + Send + 'static,
    {
        Self {
            callback: Box::new(callback),
        }
    }
}

fn on_token(sink: &mut TokenSink, token: &str) -> bool {
    (sink.callback)(token)
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
        let mut sink = TokenSink::new(callback);
        self.inner
            .pin_mut()
            .generate(prompt, system_prompt, &config, &mut sink)
    }

    pub fn clear_kv_cache(&mut self) {
        self.inner.pin_mut().clear_kv_cache();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_sink_can_cancel_streaming() {
        let mut seen = Vec::new();
        let mut sink = TokenSink::new(move |token| {
            seen.push(token.to_owned());
            token != "stop"
        });

        assert!(on_token(&mut sink, "one"));
        assert!(!on_token(&mut sink, "stop"));
    }

    #[test]
    fn generation_config_is_plain_data() {
        let config = GenerationConfig {
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 256,
            n_ctx: 8_192,
            n_batch: 512,
            n_threads: 8,
            n_gpu_layers: -1,
        };
        assert_eq!(config.max_tokens, 256);
        assert_eq!(config.n_ctx, 8_192);
    }
}
