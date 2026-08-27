from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if old not in text:
        raise SystemExit(f"anchor not found in {path}: {old[:80]!r}")
    file.write_text(text.replace(old, new, 1))


# Frontend attachment model: keep raw media ephemeral and outside persisted chat text.
replace_once(
    "src/lib/chat.ts",
    '''export interface AttachmentDraft {
  id: string;
  name: string;
  size: number;
  type: string;
  kind: "text" | "image" | "pdf" | "binary";
  contentPreview: string | null;
}
''',
    '''export interface AttachmentDraft {
  id: string;
  name: string;
  size: number;
  type: string;
  kind: "text" | "image" | "pdf" | "binary";
  contentPreview: string | null;
  mediaDataUrl: string | null;
}

export interface InferenceMediaDraft {
  kind: "image";
  name: string;
  mimeType: string;
  dataUrl: string;
}
''',
)
replace_once(
    "src/lib/chat.ts",
    '''  let contentPreview: string | null = null;
  if (isTextLike && file.size <= maxPreviewBytes) {''',
    '''  let contentPreview: string | null = null;
  let mediaDataUrl: string | null = null;
  if (isTextLike && file.size <= maxPreviewBytes) {''',
)
replace_once(
    "src/lib/chat.ts",
    '''  } else if (kind === "image") {
    contentPreview = `[Image attached: ${file.name}. Exact visual analysis requires the local OpenMindAI Lens vision runtime. Do not infer unseen image content from metadata alone.]`;
  } else if (kind === "pdf") {''',
    '''  } else if (kind === "image") {
    const maxVisionBytes = 8 * 1024 * 1024;
    if (file.size > maxVisionBytes) {
      throw new Error(`Image ${file.name} exceeds the 8 MB local vision limit.`);
    }
    const mimeType = normalizedImageMimeType(file.type, file.name);
    if (!mimeType) {
      throw new Error(`Image ${file.name} uses an unsupported format. Use PNG, JPEG, WebP, or GIF.`);
    }
    mediaDataUrl = await readFileAsDataUrl(file);
    if (!mediaDataUrl.startsWith(`data:${mimeType};base64,`)) {
      throw new Error(`Image ${file.name} could not be encoded safely for local vision.`);
    }
    contentPreview = `[Image attached: ${file.name}. Original image bytes are supplied only to the current local vision request and are not stored in chat history.]`;
  } else if (kind === "pdf") {''',
)
replace_once(
    "src/lib/chat.ts",
    '''    kind,
    contentPreview,
  };
}

export function buildMessageContent''',
    '''    kind,
    contentPreview,
    mediaDataUrl,
  };
}

export function attachmentMedia(attachments: AttachmentDraft[]): InferenceMediaDraft[] {
  return attachments.flatMap((attachment) => {
    if (attachment.kind !== "image" || !attachment.mediaDataUrl) return [];
    const mimeType = normalizedImageMimeType(attachment.type, attachment.name);
    if (!mimeType) return [];
    return [
      {
        kind: "image" as const,
        name: attachment.name,
        mimeType,
        dataUrl: attachment.mediaDataUrl,
      },
    ];
  });
}

function normalizedImageMimeType(type: string, name: string): string | null {
  const normalized = type.toLowerCase();
  if (["image/png", "image/jpeg", "image/webp", "image/gif"].includes(normalized)) {
    return normalized;
  }
  if (/\\.png$/i.test(name)) return "image/png";
  if (/\\.jpe?g$/i.test(name)) return "image/jpeg";
  if (/\\.webp$/i.test(name)) return "image/webp";
  if (/\\.gif$/i.test(name)) return "image/gif";
  return null;
}

function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("Could not read image attachment."));
    reader.onload = () => {
      if (typeof reader.result === "string") resolve(reader.result);
      else reject(new Error("Image attachment did not produce a data URL."));
    };
    reader.readAsDataURL(file);
  });
}

export function buildMessageContent''',
)
replace_once(
    "src/lib/chat.ts",
    '''  if (file.type.startsWith("image/")) return "image";''',
    '''  if (file.type.startsWith("image/") || /\\.(png|jpe?g|webp|gif)$/i.test(file.name)) return "image";''',
)
replace_once(
    "src/lib/chat.ts",
    '''    case "vision":
      return "[Mode: Image/Vision Review]\\nUse the local vision runtime when the attached image has been made available to it. Never invent visual details from file metadata. If the vision runtime is unavailable, say so clearly.";''',
    '''    case "vision":
      return "[Mode: Image/Vision Review]\\nAnalyze the attached image using the local vision runtime. Base visual claims only on image input actually supplied to the model. If the runtime rejects or cannot access the media, state that clearly rather than guessing.";''',
)

# Frontend API passes ephemeral media separately from persisted message text.
replace_once(
    "src/api.ts",
    '''  sendChatMessage: (conversationId: string, content: string, mode: string) =>
    call<Message>("send_chat_message", { conversationId, content, mode }),''',
    '''  sendChatMessage: (
    conversationId: string,
    content: string,
    mode: string,
    media: Array<{ kind: "image"; name: string; mimeType: string; dataUrl: string }> = [],
  ) => call<Message>("send_chat_message", { conversationId, content, mode, media }),''',
)

replace_once(
    "src/App.tsx",
    '''  buildMessageContent,
  inferChatMode,''',
    '''  attachmentMedia,
  buildMessageContent,
  inferChatMode,''',
)
replace_once(
    "src/App.tsx",
    '''    const inferredMode = inferChatMode(content, attachments);
    const messageContent = buildMessageContent(content, attachments, inferredMode);
    setPrompt("");''',
    '''    const inferredMode = inferChatMode(content, attachments);
    const messageContent = buildMessageContent(content, attachments, inferredMode);
    const inferenceMedia = attachmentMedia(attachments);
    setPrompt("");''',
)
replace_once(
    "src/App.tsx",
    '''      const assistant = await api.sendChatMessage(conversationId, messageContent, inferredMode);''',
    '''      const assistant = await api.sendChatMessage(
        conversationId,
        messageContent,
        inferredMode,
        inferenceMedia,
      );''',
)

# Rust inference layer validates media and builds OpenAI-compatible multimodal content.
replace_once(
    "src-tauri/src/inference.rs",
    '''const WEB_SEARCH_TIMEOUT_SECS: u64 = 12;
''',
    '''const WEB_SEARCH_TIMEOUT_SECS: u64 = 12;
const MAX_INFERENCE_MEDIA_ITEMS: usize = 4;
const MAX_MEDIA_DATA_URL_BYTES: usize = 12 * 1024 * 1024;
''',
)
replace_once(
    "src-tauri/src/inference.rs",
    '''#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceMetrics {
    pub time_to_first_token_ms: Option<u128>,
    pub generated_chars: usize,
    pub elapsed_ms: u128,
}
''',
    '''#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceMetrics {
    pub time_to_first_token_ms: Option<u128>,
    pub generated_chars: usize,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceMedia {
    pub kind: String,
    pub name: String,
    pub mime_type: String,
    pub data_url: String,
}
''',
)
replace_once(
    "src-tauri/src/inference.rs",
    '''    pub assistant: &'a Message,
    pub mode: InferenceMode,
}''',
    '''    pub assistant: &'a Message,
    pub mode: InferenceMode,
    pub media: &'a [InferenceMedia],
}''',
)
replace_once(
    "src-tauri/src/inference.rs",
    '''    let mut messages = build_context(request.database, request.conversation_id)?;
    append_live_web_context(request.client, &mut messages, &cancellation).await;
    let config = ChatGenerationConfig::default();''',
    '''    let mut messages = build_context(request.database, request.conversation_id)?;
    append_live_web_context(request.client, &mut messages, &cancellation).await;
    attach_media_to_latest_user_message(&mut messages, request.media)?;
    let config = ChatGenerationConfig::default();''',
)
replace_once(
    "src-tauri/src/inference.rs",
    '''async fn append_live_web_context(
''',
    '''fn attach_media_to_latest_user_message(
    messages: &mut [serde_json::Value],
    media: &[InferenceMedia],
) -> Result<(), AppError> {
    if media.is_empty() {
        return Ok(());
    }
    if media.len() > MAX_INFERENCE_MEDIA_ITEMS {
        return Err(AppError::InferenceFailed(format!(
            "at most {MAX_INFERENCE_MEDIA_ITEMS} image attachments can be analyzed at once"
        )));
    }

    let message = messages
        .iter_mut()
        .rev()
        .find(|message| message.get("role").and_then(|value| value.as_str()) == Some("user"))
        .ok_or_else(|| AppError::InferenceFailed("vision request has no user message".to_string()))?;
    let text = message
        .get("content")
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::InferenceFailed("vision request user message is not text".to_string()))?
        .to_string();

    let mut content = vec![json!({ "type": "text", "text": text })];
    for item in media {
        if item.kind != "image" {
            return Err(AppError::InferenceFailed(
                "unsupported inference media type".to_string(),
            ));
        }
        if item.name.trim().is_empty() || item.name.len() > 255 {
            return Err(AppError::InferenceFailed(
                "invalid image attachment name".to_string(),
            ));
        }
        if !matches!(
            item.mime_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) {
            return Err(AppError::InferenceFailed(format!(
                "unsupported image MIME type: {}",
                item.mime_type
            )));
        }
        if item.data_url.len() > MAX_MEDIA_DATA_URL_BYTES {
            return Err(AppError::InferenceFailed(
                "image attachment is too large for local vision inference".to_string(),
            ));
        }
        let expected_prefix = format!("data:{};base64,", item.mime_type);
        if !item.data_url.starts_with(&expected_prefix) {
            return Err(AppError::InferenceFailed(
                "image attachment is not a valid supported data URL".to_string(),
            ));
        }
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": item.data_url }
        }));
    }
    message["content"] = serde_json::Value::Array(content);
    Ok(())
}

async fn append_live_web_context(
''',
)

# Backend command/routing: no text-only fallback for vision, no media persistence.
replace_once(
    "src-tauri/src/lib.rs",
    '''use inference::{ActiveGenerations, InferenceMode, StreamRequest, StreamStartedEvent};''',
    '''use inference::{
    ActiveGenerations, InferenceMedia, InferenceMode, StreamRequest, StreamStartedEvent,
};''',
)
replace_once(
    "src-tauri/src/lib.rs",
    '''    let models = registry.discover_gguf_models()?;

    let active_model_id = repo''',
    '''    let models = registry.discover_gguf_models()?;

    if mode.eq_ignore_ascii_case("vision") {
        let lens = model_catalog::entry_by_id("qwen25-vl-3b-q4km")?;
        if model_catalog::dependency_path(&state.root, &lens, "mmproj").is_none() {
            return Err(app_error::AppError::ModelNotFound(
                "OpenMindAI Lens is incomplete. Download or repair the Lens model package before analyzing images."
                    .to_string(),
            ));
        }
    }

    let active_model_id = repo''',
)
replace_once(
    "src-tauri/src/lib.rs",
    '''        .ok_or_else(|| {
            app_error::AppError::ModelNotFound(
                "OpenMindAI Core is not installed. Download it from Settings > Models first."
                    .to_string(),
            )
        })?;''',
    '''        .ok_or_else(|| {
            let message = if mode.eq_ignore_ascii_case("vision") {
                "OpenMindAI Lens is not installed. Download it from Settings > Models first."
            } else {
                "OpenMindAI Core is not installed. Download it from Settings > Models first."
            };
            app_error::AppError::ModelNotFound(message.to_string())
        })?;''',
)
replace_once(
    "src-tauri/src/lib.rs",
    '''    assistant: &Message,
    mode: &str,
) -> Result<(), app_error::AppError> {''',
    '''    assistant: &Message,
    mode: &str,
    media: &[InferenceMedia],
) -> Result<(), app_error::AppError> {''',
)
replace_once(
    "src-tauri/src/lib.rs",
    '''        assistant,
        mode: inference_mode,
    })''',
    '''        assistant,
        mode: inference_mode,
        media,
    })''',
)
replace_once(
    "src-tauri/src/lib.rs",
    '''    content: String,
    mode: String,
    state: State<'_, AppState>,
) -> Result<Message, app_error::AppError> {''',
    '''    content: String,
    mode: String,
    media: Option<Vec<InferenceMedia>>,
    state: State<'_, AppState>,
) -> Result<Message, app_error::AppError> {''',
)
replace_once(
    "src-tauri/src/lib.rs",
    '''    let routing = resolve_conversation_model(&state, &conversation_id, &mode, trimmed)?;
    let model = routing.model;
''',
    '''    let media = media.unwrap_or_default();
    if !media.is_empty() && !mode.eq_ignore_ascii_case("vision") {
        return Err(app_error::AppError::InferenceFailed(
            "image media can only be sent through vision mode".to_string(),
        ));
    }
    if mode.eq_ignore_ascii_case("vision") && media.is_empty() {
        return Err(app_error::AppError::InferenceFailed(
            "vision mode requires an attached image".to_string(),
        ));
    }

    let routing = resolve_conversation_model(&state, &conversation_id, &mode, trimmed)?;
    let model = routing.model;
''',
)
replace_once(
    "src-tauri/src/lib.rs",
    '''    run_streaming_completion(&app, &state, &conversation_id, &model, &assistant, &mode).await?;
    Ok(assistant)
}''',
    '''    run_streaming_completion(
        &app,
        &state,
        &conversation_id,
        &model,
        &assistant,
        &mode,
        &media,
    )
    .await?;
    Ok(assistant)
}''',
)
replace_once(
    "src-tauri/src/lib.rs",
    '''        let user = history[..target_index]
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .cloned()
            .ok_or_else(|| {
                app_error::AppError::internal("no preceding user message to regenerate from")
            })?;

        repo.delete_message(&assistant_message_id)?;''',
    '''        let user = history[..target_index]
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .cloned()
            .ok_or_else(|| {
                app_error::AppError::internal("no preceding user message to regenerate from")
            })?;
        if user.content.contains("[Attachment:") && user.content.contains(", image,") {
            return Err(app_error::AppError::InferenceFailed(
                "Vision regeneration requires reattaching the original image because image bytes are intentionally not stored in chat history."
                    .to_string(),
            ));
        }

        repo.delete_message(&assistant_message_id)?;''',
)
replace_once(
    "src-tauri/src/lib.rs",
    '''    run_streaming_completion(&app, &state, &conversation_id, &model, &assistant, &mode).await?;
    Ok(assistant)
}

fn select_conversation_model''',
    '''    run_streaming_completion(
        &app,
        &state,
        &conversation_id,
        &model,
        &assistant,
        &mode,
        &[],
    )
    .await?;
    Ok(assistant)
}

fn select_conversation_model''',
)
replace_once(
    "src-tauri/src/lib.rs",
    '''    if matches!(normalized_mode.as_str(), "vision") {
        if let Some(model) = model_by_repo(models, "ggml-org/Qwen2.5-VL-3B-Instruct-GGUF") {
            return Some(model);
        }
    }
''',
    '''    if normalized_mode == "vision" {
        return model_by_repo(models, "ggml-org/Qwen2.5-VL-3B-Instruct-GGUF");
    }
''',
)
