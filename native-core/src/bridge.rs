use std::sync::{mpsc::SyncSender, Mutex};

#[cxx::bridge(namespace = "openmind::native")]
pub(crate) mod ffi {
    extern "Rust" {
        type TokenSink;
        fn push_token(sink: &TokenSink, token: &[u8]);
    }

    unsafe extern "C++" {
        include!("inference.h");

        type InferenceEngine;

        fn load_model(
            model_path: &str,
            base_context_tokens: u32,
            gpu_layers: i32,
        ) -> Result<UniquePtr<InferenceEngine>>;

        fn generate_stream(
            engine: Pin<&mut InferenceEngine>,
            prompt: &str,
            system_prompt: &str,
            temperature: f32,
            top_p: f32,
            max_tokens: u32,
            sink: &TokenSink,
            on_token: fn(&TokenSink, &[u8]),
        ) -> Result<()>;
    }
}

struct SinkState {
    pending_utf8: Vec<u8>,
    tx: SyncSender<String>,
}

pub(crate) struct TokenSink {
    state: Mutex<SinkState>,
}

impl TokenSink {
    pub(crate) fn new(tx: SyncSender<String>) -> Self {
        Self {
            state: Mutex::new(SinkState {
                pending_utf8: Vec::with_capacity(16),
                tx,
            }),
        }
    }

    pub(crate) fn flush(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.pending_utf8.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(&state.pending_utf8).into_owned();
        state.pending_utf8.clear();
        if !text.is_empty() {
            let _ = state.tx.send(text);
        }
    }
}

pub(crate) fn push_token(sink: &TokenSink, token: &[u8]) {
    let Ok(mut state) = sink.state.lock() else {
        return;
    };
    state.pending_utf8.extend_from_slice(token);

    loop {
        match std::str::from_utf8(&state.pending_utf8) {
            Ok(text) => {
                if !text.is_empty() {
                    let chunk = text.to_owned();
                    state.pending_utf8.clear();
                    let _ = state.tx.send(chunk);
                }
                return;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    let chunk = String::from_utf8_lossy(&state.pending_utf8[..valid]).into_owned();
                    state.pending_utf8.drain(..valid);
                    if !chunk.is_empty() {
                        let _ = state.tx.send(chunk);
                    }
                    continue;
                }

                // error_len == None means the bytes are a valid prefix of a
                // multi-byte UTF-8 scalar; wait for the next llama token.
                if error.error_len().is_none() {
                    return;
                }

                // A complete invalid sequence should never be emitted by a
                // well-formed tokenizer, but avoid wedging the stream if a
                // custom vocabulary does so.
                let lossy = String::from_utf8_lossy(&state.pending_utf8).into_owned();
                state.pending_utf8.clear();
                if !lossy.is_empty() {
                    let _ = state.tx.send(lossy);
                }
                return;
            }
        }
    }
}
