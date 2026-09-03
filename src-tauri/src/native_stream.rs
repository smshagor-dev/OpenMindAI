use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use crate::{
    app_error::AppError,
    chat::{ChatRepository, Message},
    database::Database,
    inference::{ActiveGenerations, InferenceMetrics, StreamChunkEvent, StreamDoneEvent},
    native_bridge::{ChatMessage, GenerationConfig},
    native_inference::InferenceRequest,
    native_supervisor::{NativeInferenceSupervisor, NativeModelSpec, NativeSupervisorError},
};

const UI_STREAM_CHUNK_BYTES: usize = 32;
const DB_STREAM_FLUSH_BYTES: usize = 2_048;
const TOKEN_CHANNEL_CAPACITY: usize = 64;
const MAX_CONTEXT_MESSAGES: usize = 18;
const MAX_CONTEXT_CHARS: usize = 24_000;

pub struct NativeStreamRequest<'a> {
    pub app: &'a AppHandle,
    pub database: &'a Mutex<Database>,
    pub active: &'a ActiveGenerations,
    pub supervisor: Arc<NativeInferenceSupervisor>,
    pub model: NativeModelSpec,
    pub conversation_id: &'a str,
    pub assistant: &'a Message,
    pub config: GenerationConfig,
}

#[derive(Debug)]
pub struct NativeStreamError {
    pub error: AppError,
    pub emitted_output: bool,
}

impl NativeStreamError {
    pub fn can_retry(&self) -> bool {
        !self.emitted_output
            && !matches!(
                &self.error,
                AppError::PersonalizationRejected(_)
                    | AppError::InferenceTimeout(_)
                    | AppError::InferenceCancelled(_)
                    | AppError::ContextOverflow(_)
                    | AppError::ModelOutOfMemory(_)
            )
    }

    fn before_output(error: AppError) -> Self {
        Self {
            error,
            emitted_output: false,
        }
    }

    fn after_output(error: AppError) -> Self {
        Self {
            error,
            emitted_output: true,
        }
    }
}

// Release the conversation on every exit, including DB/event errors and a
// dropped request future. Cancelling also stops a worker whose consumer left.
struct ActiveNativeGeneration<'a> {
    active: &'a ActiveGenerations,
    conversation_id: &'a str,
    cancellation: CancellationToken,
}

impl Drop for ActiveNativeGeneration<'_> {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.active.finish(self.conversation_id);
    }
}

async fn next_token(
    receiver: &mut tokio::sync::mpsc::Receiver<String>,
    cancellation: &CancellationToken,
) -> Option<String> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            receiver.close();
            None
        },
        token = receiver.recv() => token,
    }
}

pub async fn stream_native_completion(
    request: NativeStreamRequest<'_>,
) -> Result<InferenceMetrics, NativeStreamError> {
    let messages = build_text_context(request.database, request.conversation_id)
        .map_err(NativeStreamError::before_output)?;
    let inference_request = InferenceRequest::from_messages(messages).with_config(request.config);
    inference_request.validate().map_err(|error| {
        NativeStreamError::before_output(AppError::InferenceFailed(error.to_string()))
    })?;

    let cancellation = request
        .active
        .start(request.conversation_id)
        .map_err(NativeStreamError::before_output)?;
    let _active_generation = ActiveNativeGeneration {
        active: request.active,
        conversation_id: request.conversation_id,
        cancellation: cancellation.clone(),
    };
    let started = Instant::now();
    let (token_tx, mut token_rx) = tokio::sync::mpsc::channel::<String>(TOKEN_CHANNEL_CAPACITY);
    let supervisor = Arc::clone(&request.supervisor);
    let model = request.model.clone();
    let worker_cancellation = cancellation.clone();
    let worker = tokio::task::spawn_blocking(move || {
        supervisor.generate(
            model,
            inference_request,
            worker_cancellation,
            Box::new(move |token| {
                token.is_empty() || token_tx.blocking_send(token.to_string()).is_ok()
            }),
        )
    });

    let mut flush_buffer = String::new();
    let mut ui_buffer = String::new();
    let mut generated_chars = 0usize;
    let mut first_token_at = None;
    let deadline = started + Duration::from_millis(u64::from(request.config.timeout_ms));
    let mut timed_out = false;

    loop {
        let token = match tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            next_token(&mut token_rx, &cancellation),
        )
        .await
        {
            Ok(Some(token)) => token,
            Ok(None) => break,
            Err(_) => {
                timed_out = true;
                cancellation.cancel();
                break;
            }
        };
        if token.is_empty() {
            continue;
        }
        first_token_at.get_or_insert_with(|| started.elapsed().as_millis());
        generated_chars += token.chars().count();
        flush_buffer.push_str(&token);
        ui_buffer.push_str(&token);

        if ui_buffer.len() >= UI_STREAM_CHUNK_BYTES {
            if let Err(error) = emit_stream_chunk(&request, &mut ui_buffer) {
                cancellation.cancel();
                return Err(NativeStreamError {
                    error,
                    emitted_output: generated_chars > 0,
                });
            }
        }
        if flush_buffer.len() >= DB_STREAM_FLUSH_BYTES {
            if let Err(error) = flush(request.database, &request.assistant.id, &mut flush_buffer) {
                cancellation.cancel();
                return Err(NativeStreamError {
                    error,
                    emitted_output: generated_chars > 0,
                });
            }
        }
    }

    // A bounded sender may be blocked when Stop is pressed. Close before joining
    // the worker so blocking_send wakes even though we stopped draining tokens.
    token_rx.close();
    let joined = tokio::time::timeout(Duration::from_secs(2), worker).await;
    let worker_result = match joined {
        Ok(result) => result,
        Err(_) => {
            request.supervisor.quarantine();
            if !flush_buffer.is_empty() {
                flush(request.database, &request.assistant.id, &mut flush_buffer)
                    .map_err(NativeStreamError::after_output)?;
            }
            if !ui_buffer.is_empty() {
                emit_stream_chunk(&request, &mut ui_buffer)
                    .map_err(NativeStreamError::after_output)?;
            }
            finalize(&request, "failed")?;
            return Err(NativeStreamError {
                emitted_output: generated_chars > 0,
                error: AppError::InferenceTimeout(
                    "native worker did not stop; restart the app to release its runtime".into(),
                ),
            });
        }
    }
    .map_err(|error| NativeStreamError {
        error: AppError::InferenceFailed(format!("native inference worker join failed: {error}")),
        emitted_output: generated_chars > 0,
    })?;

    if !ui_buffer.is_empty() {
        emit_stream_chunk(&request, &mut ui_buffer).map_err(|error| NativeStreamError {
            error,
            emitted_output: generated_chars > 0,
        })?;
    }
    if !flush_buffer.is_empty() {
        flush(request.database, &request.assistant.id, &mut flush_buffer).map_err(|error| {
            NativeStreamError {
                error,
                emitted_output: generated_chars > 0,
            }
        })?;
    }

    if timed_out {
        finalize(&request, "failed")?;
        return Err(NativeStreamError {
            emitted_output: generated_chars > 0,
            error: AppError::InferenceTimeout("native generation deadline exceeded".into()),
        });
    }

    if matches!(worker_result, Err(NativeSupervisorError::TimedOut)) {
        finalize(&request, "failed")?;
        return Err(NativeStreamError {
            emitted_output: generated_chars > 0,
            error: AppError::InferenceTimeout(
                "native runtime did not stop; restart the app".into(),
            ),
        });
    }
    if cancellation.is_cancelled() || matches!(worker_result, Err(NativeSupervisorError::Cancelled))
    {
        finalize(&request, "cancelled")?;
        return Ok(InferenceMetrics {
            time_to_first_token_ms: first_token_at,
            generated_chars,
            elapsed_ms: started.elapsed().as_millis(),
        });
    }

    match worker_result {
        Ok(()) => {
            finalize(&request, "completed")?;
            Ok(InferenceMetrics {
                time_to_first_token_ms: first_token_at,
                generated_chars,
                elapsed_ms: started.elapsed().as_millis(),
            })
        }
        Err(error) if generated_chars == 0 => Err(NativeStreamError::before_output(
            map_supervisor_error(error),
        )),
        Err(error) => {
            finalize(&request, "failed")?;
            Err(NativeStreamError::after_output(map_supervisor_error(error)))
        }
    }
}

fn build_text_context(
    database: &Mutex<Database>,
    conversation_id: &str,
) -> Result<Vec<ChatMessage>, AppError> {
    let db = database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let repo = ChatRepository::new(&db);
    let mut messages = repo.list_messages(conversation_id)?;
    messages.retain(|message| {
        message.status == "completed"
            && matches!(message.role.as_str(), "system" | "user" | "assistant")
    });

    if messages.len() > MAX_CONTEXT_MESSAGES {
        let split_at = messages.len() - MAX_CONTEXT_MESSAGES;
        let system_messages: Vec<_> = messages
            .iter()
            .take(split_at)
            .filter(|message| message.role == "system")
            .cloned()
            .collect();
        messages = system_messages
            .into_iter()
            .chain(messages.into_iter().skip(split_at))
            .collect();
    }

    let estimated_chars: usize = messages.iter().map(|message| message.content.len()).sum();
    if estimated_chars > MAX_CONTEXT_CHARS {
        return Err(AppError::ContextOverflow(
            "conversation context is too large for the initial 8K target".to_string(),
        ));
    }

    let mut prepared = Vec::with_capacity(messages.len());
    for message in messages {
        if message.role == "user" && message.content.to_ascii_lowercase().contains("data:image/") {
            return Err(AppError::InferenceFailed(
                "native text inference does not accept vision history yet".to_string(),
            ));
        }
        if message.content.trim().is_empty() {
            continue;
        }
        prepared.push(ChatMessage::new(message.role, message.content));
    }
    if prepared.is_empty() {
        return Err(AppError::InferenceFailed(
            "conversation has no completed text context".to_string(),
        ));
    }
    Ok(prepared)
}

fn emit_stream_chunk(
    request: &NativeStreamRequest<'_>,
    buffer: &mut String,
) -> Result<(), AppError> {
    if buffer.is_empty() {
        return Ok(());
    }
    request
        .app
        .emit(
            "inference:chunk",
            StreamChunkEvent {
                conversation_id: request.conversation_id.to_string(),
                message_id: request.assistant.id.clone(),
                chunk: std::mem::take(buffer),
            },
        )
        .map_err(|error| AppError::StreamFailed(error.to_string()))
}

fn flush(
    database: &Mutex<Database>,
    assistant_id: &str,
    buffer: &mut String,
) -> Result<(), AppError> {
    let db = database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).append_message_chunk(assistant_id, buffer)?;
    buffer.clear();
    Ok(())
}

fn finalize(request: &NativeStreamRequest<'_>, status: &str) -> Result<(), NativeStreamError> {
    {
        let db = request.database.lock().map_err(|_| {
            NativeStreamError::after_output(AppError::internal("database lock poisoned"))
        })?;
        ChatRepository::new(&db)
            .set_message_status(&request.assistant.id, status)
            .map_err(NativeStreamError::after_output)?;
    }
    request
        .app
        .emit(
            "inference:done",
            StreamDoneEvent {
                conversation_id: request.conversation_id.to_string(),
                message_id: request.assistant.id.clone(),
                status: status.to_string(),
            },
        )
        .map_err(|error| {
            NativeStreamError::after_output(AppError::StreamFailed(error.to_string()))
        })?;
    Ok(())
}

fn map_supervisor_error(error: NativeSupervisorError) -> AppError {
    match error {
        NativeSupervisorError::TimedOut => AppError::InferenceTimeout(error.to_string()),
        NativeSupervisorError::Cancelled => {
            AppError::InferenceCancelled("generation cancelled".to_string())
        }
        NativeSupervisorError::Busy => {
            AppError::InferenceServerUnavailable("native inference worker is busy".to_string())
        }
        NativeSupervisorError::Stopped | NativeSupervisorError::WorkerDisconnected => {
            AppError::InferenceServerUnavailable(error.to_string())
        }
        NativeSupervisorError::Inference(error) => {
            let message = error.to_string();
            if message.contains("adapter") {
                AppError::PersonalizationRejected(message)
            } else if message.contains("deadline exceeded") {
                AppError::InferenceTimeout(message)
            } else if message.contains("KV memory budget exceeded") {
                AppError::ModelOutOfMemory(message)
            } else if message.contains("context limit exceeded")
                || message.contains("resource limit exceeded")
            {
                AppError::ContextOverflow(message)
            } else {
                AppError::InferenceFailed(message)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[test]
    fn resource_and_timeout_errors_never_retry() {
        for error in [
            AppError::InferenceTimeout("timeout".into()),
            AppError::ContextOverflow("limit".into()),
            AppError::ModelOutOfMemory("budget".into()),
        ] {
            assert!(!NativeStreamError::before_output(error).can_retry());
        }
        assert!(NativeStreamError::before_output(AppError::InferenceFailed(
            "GPU unavailable".into()
        ))
        .can_retry());
        assert!(!NativeStreamError::after_output(AppError::InferenceFailed(
            "GPU unavailable".into()
        ))
        .can_retry());
    }

    #[tokio::test]
    async fn stop_unblocks_a_full_token_queue() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        sender.send("first".to_string()).await.unwrap();
        let producer = tokio::task::spawn_blocking(move || sender.blocking_send("second".into()));
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        assert!(next_token(&mut receiver, &cancellation).await.is_none());
        assert!(timeout(Duration::from_secs(2), producer)
            .await
            .expect("producer stayed blocked after Stop")
            .unwrap()
            .is_err());
    }

    #[tokio::test]
    async fn stop_wakes_receiver_before_first_token() {
        let (_sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let cancellation = CancellationToken::new();
        let cancel = cancellation.clone();
        let stop = async move {
            tokio::task::yield_now().await;
            cancel.cancel();
        };
        let wait = async {
            assert!(next_token(&mut receiver, &cancellation).await.is_none());
        };
        timeout(Duration::from_secs(2), async {
            tokio::join!(stop, wait);
        })
        .await
        .expect("receiver waited for output after Stop");
    }

    #[tokio::test]
    async fn dropped_request_releases_conversation_and_cancels_worker() {
        let active = Arc::new(ActiveGenerations::default());
        let cancellation = active.start("conversation").unwrap();
        let worker_cancellation = cancellation.clone();
        let task_active = Arc::clone(&active);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let request = tokio::spawn(async move {
            let _guard = ActiveNativeGeneration {
                active: &task_active,
                conversation_id: "conversation",
                cancellation,
            };
            started_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        started_rx.await.unwrap();
        assert!(!active.is_idle());
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        assert!(worker_cancellation.is_cancelled());
        assert!(active.is_idle());
        assert!(active.start("conversation").is_ok());
    }
}
