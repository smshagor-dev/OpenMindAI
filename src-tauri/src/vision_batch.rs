use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::{Client, StatusCode};
use serde_json::json;

use crate::{app_error::AppError, inference::InferenceMedia};

const MAX_VISION_DATA_URL_CHARS: usize = 6_000_000;
const MAX_RESPONSE_TOKENS: u32 = 2048;
const RETRY_ATTEMPTS: u32 = 5;
const RETRY_DELAY_MS: u64 = 600;

pub async fn analyze_image(
    client: &Client,
    endpoint: &str,
    prompt: &str,
    media: &InferenceMedia,
) -> Result<String, AppError> {
    validate_media(media)?;
    let body = json!({
        "model": "openmindai-lens",
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": prompt },
                { "type": "image_url", "image_url": { "url": media.data_url } }
            ]
        }],
        "stream": false,
        "temperature": 0.0,
        "top_p": 0.9,
        "max_tokens": MAX_RESPONSE_TOKENS,
        "chat_template_kwargs": { "enable_thinking": false }
    });

    let url = format!("{endpoint}/v1/chat/completions");
    let mut attempt = 0;
    let response = loop {
        let response = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::InferenceServerUnavailable(error.to_string()))?;
        if response.status() == StatusCode::SERVICE_UNAVAILABLE && attempt < RETRY_ATTEMPTS {
            attempt += 1;
            tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
            continue;
        }
        break response
            .error_for_status()
            .map_err(|error| AppError::InferenceFailed(error.to_string()))?;
    };

    let value: serde_json::Value = response.json().await.map_err(|error| {
        AppError::InferenceFailed(format!("Lens response was invalid: {error}"))
    })?;
    let content = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(extract_text_content)
        .unwrap_or_default()
        .trim()
        .to_string();
    if content.is_empty() {
        return Err(AppError::InferenceFailed(
            "OpenMindAI Lens returned no readable page content.".to_string(),
        ));
    }
    Ok(content)
}

fn extract_text_content(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    let parts = value.as_array()?;
    let text = parts
        .iter()
        .filter_map(|part| {
            if part.get("type").and_then(|value| value.as_str()) == Some("text") {
                part.get("text").and_then(|value| value.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

fn validate_media(media: &InferenceMedia) -> Result<(), AppError> {
    if media.kind != "image" || media.name.trim().is_empty() || media.name.chars().count() > 255 {
        return Err(AppError::InferenceFailed(
            "invalid PDF page image metadata".to_string(),
        ));
    }
    if !matches!(media.mime_type.as_str(), "image/png" | "image/jpeg") {
        return Err(AppError::InferenceFailed(
            "PDF OCR supports PNG and JPEG page images only".to_string(),
        ));
    }
    if media.data_url.len() > MAX_VISION_DATA_URL_CHARS {
        return Err(AppError::ContextOverflow(
            "rendered PDF page exceeds the local Lens payload limit".to_string(),
        ));
    }
    let prefix = format!("data:{};base64,", media.mime_type);
    let encoded = media.data_url.strip_prefix(&prefix).ok_or_else(|| {
        AppError::InferenceFailed("rendered PDF page has an invalid data URL".to_string())
    })?;
    let bytes = STANDARD.decode(encoded).map_err(|error| {
        AppError::InferenceFailed(format!("rendered PDF page could not be decoded: {error}"))
    })?;
    let signature_ok = match media.mime_type.as_str() {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        _ => false,
    };
    if !signature_ok {
        return Err(AppError::InferenceFailed(
            "rendered PDF page bytes do not match their declared MIME type".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mismatched_page_payload() {
        let media = InferenceMedia {
            kind: "image".to_string(),
            name: "page 1".to_string(),
            mime_type: "image/png".to_string(),
            data_url: "data:image/png;base64,bm90LXBuZw==".to_string(),
        };
        assert!(validate_media(&media).is_err());
    }
}
