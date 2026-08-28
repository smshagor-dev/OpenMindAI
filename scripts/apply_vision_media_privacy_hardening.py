from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"anchor not found in {path}: {old[:120]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


def replace_between(path: str, start: str, end: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    start_index = text.find(start)
    if start_index < 0:
        raise SystemExit(f"start anchor not found in {path}: {start!r}")
    end_index = text.find(end, start_index)
    if end_index < 0:
        raise SystemExit(f"end anchor not found in {path}: {end!r}")
    file.write_text(text[:start_index] + new + text[end_index:], encoding="utf-8")


# Frontend: keep optimized image bytes ephemeral. Persist only attachment metadata/text.
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
  mediaMimeType: "image/png" | "image/jpeg" | null;
}

export interface InferenceMediaDraft {
  kind: "image";
  name: string;
  mimeType: "image/png" | "image/jpeg";
  dataUrl: string;
}
''',
)

replace_between(
    "src/lib/chat.ts",
    "async function encodeVisionImage(file: File) {",
    "\n\nexport function settledValue",
    '''async function encodeVisionImage(file: File) {
  const mimeType = visionMimeType(file);
  if (!mimeType) {
    throw new Error("OpenMindAI Lens currently accepts PNG, JPEG, and WebP images.");
  }
  if (file.size > MAX_VISION_IMAGE_INPUT_BYTES) {
    throw new Error("Image is too large for local vision. Use an image smaller than 16 MB.");
  }

  const objectUrl = window.URL.createObjectURL(file);
  try {
    const image = document.createElement("img");
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error("Could not decode the selected image."));
      image.src = objectUrl;
    });

    const scale = Math.min(
      1,
      MAX_VISION_IMAGE_DIMENSION / Math.max(image.naturalWidth, image.naturalHeight, 1),
    );
    const width = Math.max(1, Math.round(image.naturalWidth * scale));
    const height = Math.max(1, Math.round(image.naturalHeight * scale));
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Could not prepare the image for local vision.");
    context.drawImage(image, 0, 0, width, height);

    let mediaMimeType: "image/png" | "image/jpeg" =
      mimeType === "image/png" ? "image/png" : "image/jpeg";
    let dataUrl =
      mediaMimeType === "image/png"
        ? canvas.toDataURL("image/png")
        : canvas.toDataURL("image/jpeg", 0.88);

    if (dataUrl.length > MAX_VISION_IMAGE_DATA_URL_CHARS) {
      const flattened = document.createElement("canvas");
      flattened.width = width;
      flattened.height = height;
      const flattenedContext = flattened.getContext("2d");
      if (!flattenedContext) throw new Error("Could not compress the image for local vision.");
      flattenedContext.fillStyle = "#ffffff";
      flattenedContext.fillRect(0, 0, width, height);
      flattenedContext.drawImage(canvas, 0, 0);
      mediaMimeType = "image/jpeg";
      dataUrl = flattened.toDataURL("image/jpeg", 0.82);
    }

    if (dataUrl.length > MAX_VISION_IMAGE_DATA_URL_CHARS) {
      throw new Error("Image remains too large after local optimization. Resize it and try again.");
    }

    return { dataUrl, mimeType: mediaMimeType };
  } finally {
    window.URL.revokeObjectURL(objectUrl);
  }
}''',
)

replace_once(
    "src/lib/chat.ts",
    '''  let contentPreview: string | null = null;
  if (isTextLike && file.size <= maxPreviewBytes) {''',
    '''  let contentPreview: string | null = null;
  let mediaDataUrl: string | null = null;
  let mediaMimeType: "image/png" | "image/jpeg" | null = null;
  if (isTextLike && file.size <= maxPreviewBytes) {''',
)
replace_once(
    "src/lib/chat.ts",
    '''  } else if (kind === "image") {
    contentPreview = await encodeVisionImage(file);
  } else if (kind === "pdf") {''',
    '''  } else if (kind === "image") {
    const encoded = await encodeVisionImage(file);
    mediaDataUrl = encoded.dataUrl;
    mediaMimeType = encoded.mimeType;
    contentPreview = `[Image attached: ${file.name}. Optimized image bytes are supplied only to the current local vision request and are not stored in chat history.]`;
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
    mediaMimeType,
  };
}

export function attachmentMedia(attachments: AttachmentDraft[]): InferenceMediaDraft[] {
  return attachments.flatMap((attachment) => {
    if (
      attachment.kind !== "image" ||
      !attachment.mediaDataUrl ||
      !attachment.mediaMimeType
    ) {
      return [];
    }
    return [
      {
        kind: "image" as const,
        name: attachment.name,
        mimeType: attachment.mediaMimeType,
        dataUrl: attachment.mediaDataUrl,
      },
    ];
  });
}

export function buildMessageContent''',
)
replace_once(
    "src/lib/chat.ts",
    '''    case "image":
      return "[Mode: Image Creation]\\nCreate a production-quality image generation prompt with subject, composition, style, lighting, camera/framing, colors, negative prompt, and recommended aspect ratio. Do not claim an image file was generated unless an image generator is connected.";''',
    '''    case "image":
      return "[Mode: Image Creation]\\nCreate a concise production-quality prompt for the connected local image renderer, including subject, composition, style, lighting, camera/framing, colors, and useful negative constraints. Do not claim rendering succeeded until the artifact pipeline reports a ready image.";''',
)
replace_once(
    "src/lib/chat.ts",
    '''    case "vision":
      return "[Mode: Image/Vision Review]\\nUse the local vision runtime when the attached image has been made available to it. Never invent visual details from file metadata. If the vision runtime is unavailable, say so clearly.";''',
    '''    case "vision":
      return "[Mode: Image/Vision Review]\\nAnalyze only the image bytes supplied to the current local Lens request. Never invent visual details from attachment metadata. If Lens rejects or cannot access the image, say so clearly.";''',
)

# Frontend API and caller: pass ephemeral media separately from persisted chat text.
replace_once(
    "src/api.ts",
    '''  sendChatMessage: (conversationId: string, content: string, mode: string) =>
    call<Message>("send_chat_message", { conversationId, content, mode }),''',
    '''  sendChatMessage: (
    conversationId: string,
    content: string,
    mode: string,
    media: Array<{
      kind: "image";
      name: string;
      mimeType: "image/png" | "image/jpeg";
      dataUrl: string;
    }> = [],
  ) => call<Message>("send_chat_message", { conversationId, content, mode, media }),''',
)
replace_once(
    "src/App.tsx",
    '''import {
  buildMessageContent,
  inferChatMode,''',
    '''import {
  attachmentMedia,
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

# Rust inference: validate ephemeral media and attach it only to the request payload.
replace_once(
    "src-tauri/src/inference.rs",
    '''const MAX_VISION_DATA_URL_CHARS: usize = 6_000_000;
''',
    '''const MAX_VISION_DATA_URL_CHARS: usize = 6_000_000;
const MAX_INFERENCE_MEDIA_ITEMS: usize = 4;
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

    let mut content = match message.get("content") {
        Some(serde_json::Value::String(text)) => vec![json!({
            "type": "text",
            "text": if text.trim().is_empty() { "Review the attached image." } else { text }
        })],
        Some(serde_json::Value::Array(parts)) => parts.clone(),
        _ => {
            return Err(AppError::InferenceFailed(
                "vision request user message has unsupported content".to_string(),
            ));
        }
    };

    for item in media {
        if item.kind != "image" {
            return Err(AppError::InferenceFailed(
                "unsupported inference media type".to_string(),
            ));
        }
        if item.name.trim().is_empty() || item.name.chars().count() > 255 {
            return Err(AppError::InferenceFailed(
                "invalid image attachment name".to_string(),
            ));
        }
        if !matches!(item.mime_type.as_str(), "image/png" | "image/jpeg") {
            return Err(AppError::InferenceFailed(format!(
                "unsupported image MIME type: {}",
                item.mime_type
            )));
        }
        if item.data_url.len() > MAX_VISION_DATA_URL_CHARS {
            return Err(AppError::ContextOverflow(
                "attached image exceeds the local vision payload limit".to_string(),
            ));
        }
        let expected_prefix = format!("data:{};base64,", item.mime_type);
        if !item.data_url.starts_with(&expected_prefix) {
            return Err(AppError::InferenceFailed(
                "image attachment data URL does not match its MIME type".to_string(),
            ));
        }
        validate_inline_data_image_url(&item.data_url)?;
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
replace_once(
    "src-tauri/src/inference.rs",
    '''    #[test]
    fn creates_openai_multimodal_content() {
        let value = context_message_value(
            "user",
            "describe this".to_string(),
            vec![InlineDataImage {
                data_url: "data:image/png;base64,QUJDRA==".to_string(),
            }],
            true,
        );
        assert_eq!(value["content"][0]["type"], "text");
        assert_eq!(value["content"][1]["type"], "image_url");
        assert_eq!(
            value["content"][1]["image_url"]["url"],
            "data:image/png;base64,QUJDRA=="
        );
    }
}''',
    '''    #[test]
    fn creates_openai_multimodal_content() {
        let value = context_message_value(
            "user",
            "describe this".to_string(),
            vec![InlineDataImage {
                data_url: "data:image/png;base64,QUJDRA==".to_string(),
            }],
            true,
        );
        assert_eq!(value["content"][0]["type"], "text");
        assert_eq!(value["content"][1]["type"], "image_url");
        assert_eq!(
            value["content"][1]["image_url"]["url"],
            "data:image/png;base64,QUJDRA=="
        );
    }

    #[test]
    fn attaches_ephemeral_media_without_persisted_markup() {
        let mut messages = vec![json!({
            "role": "user",
            "content": "[Attachment: screen.png, image]\\nPlease review this image."
        })];
        let media = vec![InferenceMedia {
            kind: "image".to_string(),
            name: "screen.png".to_string(),
            mime_type: "image/png".to_string(),
            data_url: "data:image/png;base64,QUJDRA==".to_string(),
        }];
        attach_media_to_latest_user_message(&mut messages, &media).unwrap();
        assert_eq!(messages[0]["content"][0]["type"], "text");
        assert_eq!(messages[0]["content"][1]["type"], "image_url");
        assert_eq!(
            messages[0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,QUJDRA=="
        );
    }
}''',
)

# Rust command layer: accept media separately, validate mode, never persist media bytes.
replace_once(
    "src-tauri/src/lib.rs",
    '''use inference::{ActiveGenerations, InferenceMode, StreamRequest, StreamStartedEvent};''',
    '''use inference::{
    ActiveGenerations, InferenceMedia, InferenceMode, StreamRequest, StreamStartedEvent,
};''',
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
    '''    run_streaming_completion(&app, &state, &conversation_id, &model, &assistant, &mode).await?;
    Ok(assistant)
}

fn select_conversation_model''',
    '''    if mode.eq_ignore_ascii_case("vision") {
        return Err(app_error::AppError::InferenceFailed(
            "Vision responses cannot be regenerated without the original ephemeral image. Reattach the image and send it again."
                .to_string(),
        ));
    }
    run_streaming_completion(
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

print("vision media privacy hardening patch applied")
