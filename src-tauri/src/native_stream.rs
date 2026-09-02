use std::sync::{Arc, Mutex};
use std::time::Instant;

use tauri::{AppHandle, Emitter};

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
            Box::new(move |token| token_tx.blocking_send(token.to_string()).is_ok()),
        )
    });

    let mut flush_buffer = String::new();
    let mut ui_buffer = String::new();
    let mut generated_chars = 0usize;
    let mut first_token_at = None;

    while let Some(token) = token_rx.recv().await {
        if cancellation.is_cancelled() {
            break;
        }
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
                request.active.finish(request.conversation_id);
                return Err(NativeStreamError {
                    error,
                    emitted_output: generated_chars > 0,
                });
            }
        }
        if flush_buffer.len() >= DB_STREAM_FLUSH_BYTES {
            if let Err(error) = flush(request.database, &request.assistant.id, &mut flush_buffer) {
                cancellation.cancel();
                request.active.finish(request.conversation_id);
                return Err(NativeStreamError {
                    error,
                    emitted_output: generated_chars > 0,
                });
            }
        }
    }

    let worker_result = worker.await.map_err(|error| NativeStreamError {
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
        Err(error) if generated_chars == 0 => {
            request.active.finish(request.conversation_id);
            Err(NativeStreamError::before_output(map_supervisor_error(
                error,
            )))
        }
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
    request.active.finish(request.conversation_id);
    Ok(())
}

fn map_supervisor_error(error: NativeSupervisorError) -> AppError {
    match error {
        NativeSupervisorError::Cancelled => {
            AppError::InferenceCancelled("generation cancelled".to_string())
        }
        NativeSupervisorError::Busy => {
            AppError::InferenceServerUnavailable("native inference worker is busy".to_string())
        }
        NativeSupervisorError::Stopped | NativeSupervisorError::WorkerDisconnected => {
            AppError::InferenceServerUnavailable(error.to_string())
        }
        NativeSupervisorError::Inference(error) => AppError::InferenceFailed(error.to_string()),
    }
}
