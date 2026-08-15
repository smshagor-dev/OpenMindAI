use std::{collections::HashMap, sync::Mutex, time::Instant};

use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use crate::{
    app_error::AppError,
    chat::{ChatRepository, Message},
    database::Database,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamChunkEvent {
    pub conversation_id: String,
    pub message_id: String,
    pub chunk: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStartedEvent {
    pub conversation_id: String,
    pub user: Message,
    pub assistant: Message,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamDoneEvent {
    pub conversation_id: String,
    pub message_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceMetrics {
    pub time_to_first_token_ms: Option<u128>,
    pub generated_chars: usize,
    pub elapsed_ms: u128,
}

#[derive(Default)]
pub struct ActiveGenerations {
    tokens: Mutex<HashMap<String, CancellationToken>>,
}

impl ActiveGenerations {
    pub fn start(&self, conversation_id: &str) -> Result<CancellationToken, AppError> {
        let mut tokens = self
            .tokens
            .lock()
            .map_err(|_| AppError::internal("generation lock poisoned"))?;
        if tokens.contains_key(conversation_id) {
            return Err(AppError::InferenceFailed(
                "one generation is already active in this conversation".to_string(),
            ));
        }
        let token = CancellationToken::new();
        tokens.insert(conversation_id.to_string(), token.clone());
        Ok(token)
    }

    pub fn cancel(&self, conversation_id: &str) -> Result<(), AppError> {
        if let Some(token) = self
            .tokens
            .lock()
            .map_err(|_| AppError::internal("generation lock poisoned"))?
            .get(conversation_id)
        {
            token.cancel();
        }
        Ok(())
    }

    pub fn finish(&self, conversation_id: &str) {
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.remove(conversation_id);
        }
    }
}

pub struct StreamRequest<'a> {
    pub app: &'a AppHandle,
    pub database: &'a Mutex<Database>,
    pub active: &'a ActiveGenerations,
    pub client: &'a Client,
    pub endpoint: &'a str,
    pub conversation_id: &'a str,
    pub assistant: &'a Message,
    pub mode: InferenceMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGenerationConfig {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub min_p: f32,
    pub max_tokens: u32,
    pub presence_penalty: f32,
}

impl Default for ChatGenerationConfig {
    fn default() -> Self {
        Self {
            temperature: 0.6,
            top_p: 0.95,
            top_k: 20,
            min_p: 0.0,
            max_tokens: 768,
            presence_penalty: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InferenceMode {
    Chat,
    Thinking,
}

pub async fn stream_chat_completion(
    request: StreamRequest<'_>,
) -> Result<InferenceMetrics, AppError> {
    let cancellation = request.active.start(request.conversation_id)?;
    let started = Instant::now();
    let messages = build_context(request.database, request.conversation_id)?;
    let config = ChatGenerationConfig::default();
    let body = json!({
        "model": "qwen3-4b-q4_k_m",
        "messages": messages,
        "stream": true,
        "temperature": config.temperature,
        "top_p": config.top_p,
        "top_k": config.top_k,
        "min_p": config.min_p,
        "max_tokens": config.max_tokens,
        "presence_penalty": config.presence_penalty,
        "chat_template_kwargs": {
            "enable_thinking": matches!(request.mode, InferenceMode::Thinking)
        }
    });

    let response =
        post_completion_with_retry(request.client, request.endpoint, &body, &cancellation).await?;

    let mut stream = response.bytes_stream();
    let mut sse_buffer = String::new();
    let mut flush_buffer = String::new();
    let mut generated_chars = 0;
    let mut first_token_at = None;
    let mut status = "completed";

    while let Some(chunk) = stream.next().await {
        if cancellation.is_cancelled() {
            status = "cancelled";
            break;
        }

        let bytes = chunk.map_err(|error| AppError::StreamFailed(error.to_string()))?;
        sse_buffer.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(index) = sse_buffer.find("\n\n") {
            let frame = sse_buffer[..index].to_string();
            sse_buffer = sse_buffer[index + 2..].to_string();
            for line in frame.lines() {
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    continue;
                }
                if let Some(token) = parse_openai_delta(data)? {
                    if token.is_empty() {
                        continue;
                    }
                    first_token_at.get_or_insert_with(|| started.elapsed().as_millis());
                    generated_chars += token.chars().count();
                    flush_buffer.push_str(&token);
                    request
                        .app
                        .emit(
                            "inference:chunk",
                            StreamChunkEvent {
                                conversation_id: request.conversation_id.to_string(),
                                message_id: request.assistant.id.clone(),
                                chunk: token,
                            },
                        )
                        .map_err(|error| AppError::StreamFailed(error.to_string()))?;

                    if flush_buffer.len() >= 512 {
                        flush(request.database, &request.assistant.id, &mut flush_buffer)?;
                    }
                }
            }
        }
    }

    if !flush_buffer.is_empty() {
        flush(request.database, &request.assistant.id, &mut flush_buffer)?;
    }
    set_status(request.database, &request.assistant.id, status)?;
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
        .map_err(|error| AppError::StreamFailed(error.to_string()))?;
    request.active.finish(request.conversation_id);

    Ok(InferenceMetrics {
        time_to_first_token_ms: first_token_at,
        generated_chars,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

/// llama-server runs with a single inference slot (`--parallel 1`), so it
/// answers 503 "no slot available" whenever another request in this app is
/// still finishing or it just swapped models — never a permanent condition.
/// Retrying briefly turns that blip into a normal short wait instead of
/// surfacing a raw HTTP 503 to the user.
const COMPLETION_RETRY_ATTEMPTS: u32 = 5;
const COMPLETION_RETRY_DELAY_MS: u64 = 600;

async fn post_completion_with_retry(
    client: &Client,
    endpoint: &str,
    body: &serde_json::Value,
    cancellation: &CancellationToken,
) -> Result<reqwest::Response, AppError> {
    let url = format!("{endpoint}/v1/chat/completions");
    let mut attempt = 0;
    loop {
        let response = client
            .post(&url)
            .json(body)
            .send()
            .await
            .map_err(|error| AppError::InferenceServerUnavailable(error.to_string()))?;

        if response.status() == StatusCode::SERVICE_UNAVAILABLE
            && attempt < COMPLETION_RETRY_ATTEMPTS
        {
            attempt += 1;
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(COMPLETION_RETRY_DELAY_MS)) => continue,
                _ = cancellation.cancelled() => {
                    return Err(AppError::InferenceCancelled("generation cancelled".to_string()));
                }
            }
        }

        return response
            .error_for_status()
            .map_err(|error| AppError::InferenceFailed(error.to_string()));
    }
}

fn build_context(
    database: &Mutex<Database>,
    conversation_id: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    let db = database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let repo = ChatRepository::new(&db);
    let mut messages = repo.list_messages(conversation_id)?;
    messages.retain(|message| {
        message.status == "completed"
            && matches!(message.role.as_str(), "system" | "user" | "assistant")
    });
    let max_messages = 18;
    if messages.len() > max_messages {
        let split_at = messages.len() - max_messages;
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
    if estimated_chars > 24_000 {
        return Err(AppError::ContextOverflow(
            "conversation context is too large for the initial 8K target".to_string(),
        ));
    }
    Ok(messages
        .into_iter()
        .map(|message| json!({ "role": message.role, "content": message.content }))
        .collect())
}

fn parse_openai_delta(data: &str) -> Result<Option<String>, AppError> {
    let value: serde_json::Value =
        serde_json::from_str(data).map_err(|error| AppError::StreamFailed(error.to_string()))?;
    Ok(value["choices"]
        .get(0)
        .and_then(|choice| choice["delta"]["content"].as_str())
        .map(ToString::to_string))
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

fn set_status(
    database: &Mutex<Database>,
    assistant_id: &str,
    status: &str,
) -> Result<(), AppError> {
    let db = database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).set_message_status(assistant_id, status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_streaming_delta() {
        let data = r#"{"choices":[{"delta":{"content":"hello"}}]}"#;
        assert_eq!(parse_openai_delta(data).unwrap(), Some("hello".to_string()));
    }

    #[test]
    fn active_generation_cancels() {
        let active = ActiveGenerations::default();
        let token = active.start("c1").unwrap();
        active.cancel("c1").unwrap();
        assert!(token.is_cancelled());
        active.finish("c1");
    }
}
