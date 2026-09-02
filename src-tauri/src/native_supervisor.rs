use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
};

use tokio_util::sync::CancellationToken;

use crate::native_inference::{
    InferenceBackend, InferenceError, InferenceRequest, NativeBackend, TokenCallback,
};

const WORKER_QUEUE_CAPACITY: usize = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeModelSpec {
    pub id: String,
    pub path: PathBuf,
    pub n_gpu_layers: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeSupervisorState {
    Unavailable,
    Idle,
    Loading {
        model_id: String,
    },
    Ready {
        model_id: String,
    },
    Generating {
        model_id: String,
    },
    Recovering {
        model_id: String,
    },
    Error {
        model_id: Option<String>,
        message: String,
    },
    Stopped,
}

#[derive(Debug)]
pub enum NativeSupervisorError {
    Busy,
    Stopped,
    Cancelled,
    Inference(InferenceError),
    WorkerDisconnected,
}

impl std::fmt::Display for NativeSupervisorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => write!(f, "native inference worker is busy"),
            Self::Stopped => write!(f, "native inference worker is stopped"),
            Self::Cancelled => write!(f, "native inference was cancelled"),
            Self::Inference(error) => error.fmt(f),
            Self::WorkerDisconnected => write!(f, "native inference worker disconnected"),
        }
    }
}

impl std::error::Error for NativeSupervisorError {}

impl From<InferenceError> for NativeSupervisorError {
    fn from(value: InferenceError) -> Self {
        Self::Inference(value)
    }
}

struct GenerationCommand {
    model: NativeModelSpec,
    request: InferenceRequest,
    cancellation: CancellationToken,
    on_token: TokenCallback,
    result: SyncSender<Result<(), NativeSupervisorError>>,
}

enum WorkerCommand {
    Generate(GenerationCommand),
    Clear,
    Shutdown,
}

struct LoadedModel {
    spec: NativeModelSpec,
    backend: NativeBackend,
}

pub struct NativeInferenceSupervisor {
    commands: SyncSender<WorkerCommand>,
    state: Arc<Mutex<NativeSupervisorState>>,
    shutdown: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl NativeInferenceSupervisor {
    pub fn start() -> Self {
        let (commands, receiver) = mpsc::sync_channel(WORKER_QUEUE_CAPACITY);
        let state = Arc::new(Mutex::new(NativeSupervisorState::Idle));
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_shutdown = Arc::clone(&shutdown);
        let worker = thread::Builder::new()
            .name("openmind-native-inference".to_string())
            .spawn(move || worker_loop(receiver, worker_state, worker_shutdown))
            .expect("failed to spawn native inference worker");

        Self {
            commands,
            state,
            shutdown,
            worker: Mutex::new(Some(worker)),
        }
    }

    pub fn state(&self) -> NativeSupervisorState {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| NativeSupervisorState::Error {
                model_id: None,
                message: "native inference state lock poisoned".to_string(),
            })
    }

    pub fn generate(
        &self,
        model: NativeModelSpec,
        request: InferenceRequest,
        cancellation: CancellationToken,
        on_token: TokenCallback,
    ) -> Result<(), NativeSupervisorError> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(NativeSupervisorError::Stopped);
        }
        if cancellation.is_cancelled() {
            return Err(NativeSupervisorError::Cancelled);
        }
        request.validate()?;

        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let command = WorkerCommand::Generate(GenerationCommand {
            model,
            request,
            cancellation,
            on_token,
            result: result_tx,
        });
        match self.commands.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(NativeSupervisorError::Busy),
            Err(TrySendError::Disconnected(_)) => {
                return Err(NativeSupervisorError::WorkerDisconnected)
            }
        }

        result_rx
            .recv()
            .map_err(|_| NativeSupervisorError::WorkerDisconnected)?
    }

    pub fn clear_context(&self) -> Result<(), NativeSupervisorError> {
        match self.commands.try_send(WorkerCommand::Clear) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(NativeSupervisorError::Busy),
            Err(TrySendError::Disconnected(_)) => Err(NativeSupervisorError::WorkerDisconnected),
        }
    }
}

impl Drop for NativeInferenceSupervisor {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.commands.try_send(WorkerCommand::Shutdown);
        if let Ok(mut worker) = self.worker.lock() {
            if let Some(handle) = worker.take() {
                let _ = handle.join();
            }
        }
        set_state(&self.state, NativeSupervisorState::Stopped);
    }
}

fn worker_loop(
    receiver: Receiver<WorkerCommand>,
    state: Arc<Mutex<NativeSupervisorState>>,
    shutdown: Arc<AtomicBool>,
) {
    let mut loaded: Option<LoadedModel> = None;
    while !shutdown.load(Ordering::Acquire) {
        let Ok(command) = receiver.recv() else {
            break;
        };
        match command {
            WorkerCommand::Generate(command) => {
                run_generation(&mut loaded, &state, &shutdown, command);
            }
            WorkerCommand::Clear => {
                if let Some(model) = loaded.as_mut() {
                    model.backend.clear_context();
                    set_state(
                        &state,
                        NativeSupervisorState::Ready {
                            model_id: model.spec.id.clone(),
                        },
                    );
                }
            }
            WorkerCommand::Shutdown => break,
        }
    }
    drop(loaded);
    set_state(&state, NativeSupervisorState::Stopped);
}

fn run_generation(
    loaded: &mut Option<LoadedModel>,
    state: &Arc<Mutex<NativeSupervisorState>>,
    shutdown: &Arc<AtomicBool>,
    command: GenerationCommand,
) {
    let GenerationCommand {
        model,
        request,
        cancellation,
        mut on_token,
        result,
    } = command;

    let model_changed = loaded.as_ref().is_none_or(|current| current.spec != model);
    if model_changed {
        if let Some(current) = loaded.as_ref() {
            set_state(
                state,
                NativeSupervisorState::Recovering {
                    model_id: current.spec.id.clone(),
                },
            );
        }
        *loaded = None;
        set_state(
            state,
            NativeSupervisorState::Loading {
                model_id: model.id.clone(),
            },
        );
        match NativeBackend::load(&model.path, model.n_gpu_layers) {
            Ok(backend) => {
                *loaded = Some(LoadedModel {
                    spec: model.clone(),
                    backend,
                });
                set_state(
                    state,
                    NativeSupervisorState::Ready {
                        model_id: model.id.clone(),
                    },
                );
            }
            Err(error) => {
                set_state(
                    state,
                    NativeSupervisorState::Error {
                        model_id: Some(model.id),
                        message: error.to_string(),
                    },
                );
                let _ = result.send(Err(NativeSupervisorError::Inference(error)));
                return;
            }
        }
    }

    if cancellation.is_cancelled() || shutdown.load(Ordering::Acquire) {
        let _ = result.send(Err(NativeSupervisorError::Cancelled));
        return;
    }

    set_state(
        state,
        NativeSupervisorState::Generating {
            model_id: model.id.clone(),
        },
    );
    let worker_shutdown = Arc::clone(shutdown);
    let worker_cancellation = cancellation.clone();
    let backend = loaded
        .as_mut()
        .expect("native model must be loaded before generation");
    let generation = backend.generate(
        request,
        Box::new(move |token| {
            if worker_shutdown.load(Ordering::Acquire) || worker_cancellation.is_cancelled() {
                return false;
            }
            on_token(token)
        }),
    );

    if cancellation.is_cancelled() || shutdown.load(Ordering::Acquire) {
        set_state(state, NativeSupervisorState::Ready { model_id: model.id });
        let _ = result.send(Err(NativeSupervisorError::Cancelled));
        return;
    }

    match generation {
        Ok(()) => {
            set_state(state, NativeSupervisorState::Ready { model_id: model.id });
            let _ = result.send(Ok(()));
        }
        Err(error) => {
            set_state(
                state,
                NativeSupervisorState::Error {
                    model_id: Some(model.id),
                    message: error.to_string(),
                },
            );
            let _ = result.send(Err(NativeSupervisorError::Inference(error)));
        }
    }
}

fn set_state(state: &Arc<Mutex<NativeSupervisorState>>, next: NativeSupervisorState) {
    if let Ok(mut current) = state.lock() {
        *current = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_starts_idle() {
        let supervisor = NativeInferenceSupervisor::start();
        assert_eq!(supervisor.state(), NativeSupervisorState::Idle);
    }

    #[test]
    fn cancelled_request_is_rejected_before_queueing() {
        let supervisor = NativeInferenceSupervisor::start();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let result = supervisor.generate(
            NativeModelSpec {
                id: "model".to_string(),
                path: PathBuf::from("missing.gguf"),
                n_gpu_layers: 0,
            },
            InferenceRequest::new("hello", ""),
            cancelled,
            Box::new(|_| true),
        );
        assert!(matches!(result, Err(NativeSupervisorError::Cancelled)));
    }
}
