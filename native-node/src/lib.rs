use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use napi::{
    bindgen_prelude::*,
    threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode},
    Status,
};
use napi_derive::napi;
use openmind_native_core::{EngineConfig, GenerateConfig, NativeLlamaEngine};

const TOKEN_CHANNEL_CAPACITY: usize = 128;

type StreamCallback = ThreadsafeFunction<
    (String, String),
    (),
    (String, String),
    Status,
    false,
    false,
    256,
>;

#[napi(object)]
pub struct NativeEngineOptions {
    pub base_context_tokens: Option<u32>,
    /// -1 = maximum GPU offload supported by the compiled llama.cpp backend.
    /// 0 = CPU only.
    pub gpu_layers: Option<i32>,
}

#[napi(object)]
pub struct NativeGenerateOptions {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<u32>,
}

enum WorkerCommand {
    Generate {
        prompt: String,
        system_prompt: String,
        config: GenerateConfig,
        events: StreamCallback,
    },
    Shutdown,
}

#[napi]
pub struct NativeLlama {
    commands: Sender<WorkerCommand>,
}

#[napi]
impl NativeLlama {
    #[napi(constructor)]
    pub fn new(model_path: String, options: Option<NativeEngineOptions>) -> Result<Self> {
        let options = options.unwrap_or(NativeEngineOptions {
            base_context_tokens: None,
            gpu_layers: None,
        });
        let config = EngineConfig {
            base_context_tokens: options.base_context_tokens.unwrap_or(4096),
            gpu_layers: options.gpu_layers.unwrap_or(-1),
        };

        let (commands_tx, commands_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);

        thread::Builder::new()
            .name("openmind-native-llama".to_string())
            .spawn(move || {
                let engine = NativeLlamaEngine::load(&model_path, config);
                match engine {
                    Ok(mut engine) => {
                        let _ = ready_tx.send(Ok::<(), String>(()));
                        worker_loop(&mut engine, commands_rx);
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                    }
                }
            })
            .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                commands: commands_tx,
            }),
            Ok(Err(error)) => Err(Error::new(Status::GenericFailure, error)),
            Err(error) => Err(Error::new(
                Status::GenericFailure,
                format!("native llama worker failed during startup: {error}"),
            )),
        }
    }

    /// Queues one generation on the dedicated native worker.
    ///
    /// `events(kind, data)` receives:
    /// - ("token", "...") for each streamed UTF-8 chunk
    /// - ("done", "") when generation finishes
    /// - ("error", "message") on failure
    #[napi]
    pub fn generate(
        &self,
        prompt: String,
        system_prompt: Option<String>,
        options: Option<NativeGenerateOptions>,
        events: StreamCallback,
    ) -> Result<()> {
        let options = options.unwrap_or(NativeGenerateOptions {
            temperature: None,
            top_p: None,
            max_tokens: None,
        });
        let config = GenerateConfig {
            temperature: options.temperature.unwrap_or(0.7) as f32,
            top_p: options.top_p.unwrap_or(0.9) as f32,
            max_tokens: options.max_tokens.unwrap_or(1024),
        };

        self.commands
            .send(WorkerCommand::Generate {
                prompt,
                system_prompt: system_prompt.unwrap_or_default(),
                config,
                events,
            })
            .map_err(|error| {
                Error::new(
                    Status::GenericFailure,
                    format!("native llama worker is unavailable: {error}"),
                )
            })
    }
}

impl Drop for NativeLlama {
    fn drop(&mut self) {
        let _ = self.commands.send(WorkerCommand::Shutdown);
    }
}

fn worker_loop(engine: &mut NativeLlamaEngine, commands: Receiver<WorkerCommand>) {
    while let Ok(command) = commands.recv() {
        match command {
            WorkerCommand::Generate {
                prompt,
                system_prompt,
                config,
                events,
            } => {
                let (token_tx, token_rx) = mpsc::sync_channel(TOKEN_CHANNEL_CAPACITY);
                let relay_events = events.clone();
                let relay = thread::spawn(move || {
                    while let Ok(token) = token_rx.recv() {
                        let status = relay_events.call(
                            ("token".to_string(), token),
                            ThreadsafeFunctionCallMode::Blocking,
                        );
                        if status == Status::Closing {
                            break;
                        }
                    }
                });

                let result = engine.generate_to_sender(
                    &prompt,
                    &system_prompt,
                    config,
                    token_tx,
                );
                let _ = relay.join();

                match result {
                    Ok(()) => {
                        let _ = events.call(
                            ("done".to_string(), String::new()),
                            ThreadsafeFunctionCallMode::Blocking,
                        );
                    }
                    Err(error) => {
                        let _ = events.call(
                            ("error".to_string(), error.to_string()),
                            ThreadsafeFunctionCallMode::Blocking,
                        );
                    }
                }
            }
            WorkerCommand::Shutdown => break,
        }
    }
}
