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

impl Default for GenerationConfig {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 512,
            n_ctx: 8_192,
            n_batch: 512,
            n_threads: 1,
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

    pub fn from_boxed(callback: Box<dyn FnMut(&str) -> bool + Send>) -> Self {
        Self { callback }
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
        self.generate_boxed(prompt, system_prompt, config, Box::new(callback))
    }

    pub fn generate_boxed(
        &mut self,
        prompt: &str,
        system_prompt: &str,
        config: GenerationConfig,
        callback: Box<dyn FnMut(&str) -> bool + Send>,
    ) -> Result<(), cxx::Exception> {
        let mut sink = TokenSink::from_boxed(callback);
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
    fn generation_config_validates_boundary_values() {
        let config = GenerationConfig {
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 256,
            n_ctx: 8_192,
            n_batch: 512,
            n_threads: 8,
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