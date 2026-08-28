from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"anchor not found in {path}: {old[:120]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


def replace_between(path: str, start: str, end: str, replacement: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    start_index = text.find(start)
    if start_index < 0:
        raise SystemExit(f"start anchor not found in {path}: {start!r}")
    end_index = text.find(end, start_index)
    if end_index < 0:
        raise SystemExit(f"end anchor not found in {path}: {end!r}")
    file.write_text(text[:start_index] + replacement + text[end_index:], encoding="utf-8")


# Frontend: normalize image attachments into bounded data URLs that render in chat
# and can be converted to llama.cpp's OpenAI-compatible multimodal content format.
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

export function settledValue''',
    '''export interface AttachmentDraft {
  id: string;
  name: string;
  size: number;
  type: string;
  kind: "text" | "image" | "pdf" | "binary";
  contentPreview: string | null;
}

const MAX_VISION_IMAGE_INPUT_BYTES = 16 * 1024 * 1024;
const MAX_VISION_IMAGE_DATA_URL_CHARS = 6_000_000;
const MAX_VISION_IMAGE_DIMENSION = 2048;
const SUPPORTED_VISION_IMAGE_TYPES = new Set(["image/jpeg", "image/png", "image/webp"]);

async function encodeVisionImage(file: File) {
  const mimeType = file.type.toLowerCase();
  if (!SUPPORTED_VISION_IMAGE_TYPES.has(mimeType)) {
    throw new Error("OpenMindAI Lens currently accepts PNG, JPEG, and WebP images.");
  }
  if (file.size > MAX_VISION_IMAGE_INPUT_BYTES) {
    throw new Error("Image is too large for local vision. Use an image smaller than 16 MB.");
  }

  const bitmap = await createImageBitmap(file);
  try {
    const scale = Math.min(
      1,
      MAX_VISION_IMAGE_DIMENSION / Math.max(bitmap.width, bitmap.height, 1),
    );
    const width = Math.max(1, Math.round(bitmap.width * scale));
    const height = Math.max(1, Math.round(bitmap.height * scale));
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Could not prepare the image for local vision.");
    context.drawImage(bitmap, 0, 0, width, height);

    let dataUrl =
      mimeType === "image/png"
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
      dataUrl = flattened.toDataURL("image/jpeg", 0.82);
    }

    if (dataUrl.length > MAX_VISION_IMAGE_DATA_URL_CHARS) {
      throw new Error("Image remains too large after local optimization. Resize it and try again.");
    }

    const safeName = file.name.replace(/[\\]\\r\\n]/g, " ").replace(/\\]/g, " ");
    return `![${safeName}](${dataUrl})`;
  } finally {
    bitmap.close();
  }
}

export function settledValue''',
)

replace_once(
    "src/lib/chat.ts",
    '''  } else if (kind === "image") {
    contentPreview = `[Image attached: ${file.name}. Exact visual analysis requires the local OpenMindAI Lens vision runtime. Do not infer unseen image content from metadata alone.]`;
  } else if (kind === "pdf") {''',
    '''  } else if (kind === "image") {
    contentPreview = await encodeVisionImage(file);
  } else if (kind === "pdf") {''',
)

replace_once(
    "src/App.tsx",
    '''  async function addFiles(files: FileList | null) {
    if (!files) return;
    const next = await Promise.all(Array.from(files).map(readAttachment));
    setAttachments((items) => [...items, ...next]);
    setView("chat");
    composerRef.current?.focus();
  }
''',
    '''  async function addFiles(files: FileList | null) {
    if (!files) return;
    try {
      const next = await Promise.all(Array.from(files).map(readAttachment));
      setAttachments((items) => [...items, ...next]);
      setView("chat");
      composerRef.current?.focus();
    } catch (caught) {
      showError(caught);
    }
  }
''',
)

# Backend: parse only safe raster data URLs, keep at most the newest image-bearing
# user turn multimodal, and strip older base64 payloads from context accounting.
replace_once(
    "src-tauri/src/inference.rs",
    '''const WEB_SEARCH_TIMEOUT_SECS: u64 = 12;
''',
    '''const WEB_SEARCH_TIMEOUT_SECS: u64 = 12;
const MAX_VISION_DATA_URL_CHARS: usize = 6_000_000;
''',
)

replace_once(
    "src-tauri/src/inference.rs",
    '''#[derive(Debug, Clone, PartialEq, Eq)]
struct WebSearchResult {
    title: String,
    url: String,
    snippet: String,
}
''',
    '''#[derive(Debug, Clone, PartialEq, Eq)]
struct WebSearchResult {
    title: String,
    url: String,
    snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlineDataImage {
    alt: String,
    data_url: String,
}
''',
)

replace_once(
    "src-tauri/src/inference.rs",
    '''fn latest_user_text(messages: &[serde_json::Value]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        if message.get("role")?.as_str()? != "user" {
            return None;
        }
        message.get("content")?.as_str().map(ToOwned::to_owned)
    })
}
''',
    '''fn latest_user_text(messages: &[serde_json::Value]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        if message.get("role")?.as_str()? != "user" {
            return None;
        }
        match message.get("content")? {
            serde_json::Value::String(content) => Some(content.clone()),
            serde_json::Value::Array(parts) => {
                let text = parts
                    .iter()
                    .filter_map(|part| {
                        if part.get("type")?.as_str()? == "text" {
                            part.get("text")?.as_str()
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                (!text.trim().is_empty()).then_some(text)
            }
            _ => None,
        }
    })
}
''',
)

replace_between(
    "src-tauri/src/inference.rs",
    "fn build_context(\n",
    "fn parse_openai_delta(",
    '''fn build_context(
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

    let mut prepared = Vec::with_capacity(messages.len());
    for message in messages {
        let (text, images) = extract_inline_data_images(&message.content)?;
        prepared.push((message, text, images));
    }
    let latest_image_turn = prepared
        .iter()
        .rposition(|(message, _, images)| message.role == "user" && !images.is_empty());
    let estimated_chars: usize = prepared.iter().map(|(_, text, _)| text.len()).sum();
    if estimated_chars > 24_000 {
        return Err(AppError::ContextOverflow(
            "conversation context is too large for the initial 8K target".to_string(),
        ));
    }

    Ok(prepared
        .into_iter()
        .enumerate()
        .map(|(index, (message, text, images))| {
            context_message_value(&message.role, text, images, Some(index) == latest_image_turn)
        })
        .collect())
}

fn context_message_value(
    role: &str,
    text: String,
    images: Vec<InlineDataImage>,
    include_images: bool,
) -> serde_json::Value {
    if !include_images || images.is_empty() {
        return json!({ "role": role, "content": text });
    }

    let mut content = Vec::with_capacity(images.len() + 1);
    let trimmed = text.trim();
    content.push(json!({
        "type": "text",
        "text": if trimmed.is_empty() { "Review the attached image." } else { trimmed }
    }));
    for image in images {
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": image.data_url }
        }));
    }
    json!({ "role": role, "content": content })
}

fn extract_inline_data_images(content: &str) -> Result<(String, Vec<InlineDataImage>), AppError> {
    let mut text = String::with_capacity(content.len().min(32_000));
    let mut images = Vec::new();
    let mut cursor = 0;

    while let Some(relative_start) = content[cursor..].find("![") {
        let start = cursor + relative_start;
        let Some(alt_end_relative) = content[start + 2..].find("](") else {
            break;
        };
        let alt_end = start + 2 + alt_end_relative;
        let url_start = alt_end + 2;
        let Some(close_relative) = content[url_start..].find(')') else {
            break;
        };
        let close = url_start + close_relative;
        let url = &content[url_start..close];
        if !url.to_ascii_lowercase().starts_with("data:image/") {
            text.push_str(&content[cursor..close + 1]);
            cursor = close + 1;
            continue;
        }

        validate_inline_data_image_url(url)?;
        text.push_str(&content[cursor..start]);
        let alt = content[start + 2..alt_end]
            .chars()
            .map(|character| {
                if matches!(character, '\\r' | '\\n') {
                    ' '
                } else {
                    character
                }
            })
            .collect::<String>();
        let alt = alt.trim().to_string();
        if alt.is_empty() {
            text.push_str("[Attached image]");
        } else {
            text.push_str(&format!("[Attached image: {alt}]"));
        }
        images.push(InlineDataImage {
            alt,
            data_url: url.to_string(),
        });
        cursor = close + 1;
    }
    text.push_str(&content[cursor..]);
    Ok((text, images))
}

fn validate_inline_data_image_url(url: &str) -> Result<(), AppError> {
    if url.len() > MAX_VISION_DATA_URL_CHARS {
        return Err(AppError::ContextOverflow(
            "attached image exceeds the local vision payload limit".to_string(),
        ));
    }
    let (metadata, payload) = url.split_once(',').ok_or_else(|| {
        AppError::InferenceFailed("attached image data URL is malformed".to_string())
    })?;
    let metadata = metadata.to_ascii_lowercase();
    if !matches!(
        metadata.as_str(),
        "data:image/jpeg;base64"
            | "data:image/jpg;base64"
            | "data:image/png;base64"
            | "data:image/webp;base64"
    ) {
        return Err(AppError::InferenceFailed(
            "local vision accepts PNG, JPEG, and WebP image data only".to_string(),
        ));
    }
    if payload.is_empty()
        || !payload
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return Err(AppError::InferenceFailed(
            "attached image contains invalid base64 data".to_string(),
        ));
    }
    Ok(())
}

''',
)

replace_once(
    "src-tauri/src/inference.rs",
    '''    #[test]
    fn rejects_private_search_urls() {
        assert!(!is_public_web_url("http://127.0.0.1/admin"));
        assert!(!is_public_web_url("http://192.168.1.20/"));
        assert!(is_public_web_url("https://example.com/article"));
    }
''',
    '''    #[test]
    fn rejects_private_search_urls() {
        assert!(!is_public_web_url("http://127.0.0.1/admin"));
        assert!(!is_public_web_url("http://192.168.1.20/"));
        assert!(is_public_web_url("https://example.com/article"));
    }

    #[test]
    fn extracts_safe_inline_vision_image() {
        let (text, images) = extract_inline_data_images(
            "review this\\n![screen.png](data:image/png;base64,QUJDRA==)",
        )
        .unwrap();
        assert!(text.contains("[Attached image: screen.png]"));
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].alt, "screen.png");
        assert_eq!(images[0].data_url, "data:image/png;base64,QUJDRA==");
    }

    #[test]
    fn rejects_unsupported_inline_vision_image() {
        let error = extract_inline_data_images(
            "![vector.svg](data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=)",
        )
        .unwrap_err();
        assert!(matches!(error, AppError::InferenceFailed(_)));
    }

    #[test]
    fn creates_openai_multimodal_content() {
        let value = context_message_value(
            "user",
            "describe this".to_string(),
            vec![InlineDataImage {
                alt: "screen.png".to_string(),
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
''',
)

# Routing: vision is a hard requirement for Lens, never a text-model fallback.
replace_once(
    "src-tauri/src/lib.rs",
    '''    let selected = select_conversation_model(&models, active_model_id.as_deref(), mode, content)
        .ok_or_else(|| {
            app_error::AppError::ModelNotFound(
                "OpenMindAI Core is not installed. Download it from Settings > Models first."
                    .to_string(),
            )
        })?;
''',
    '''    let selected = select_conversation_model(&models, active_model_id.as_deref(), mode, content)
        .ok_or_else(|| {
            let message = if mode.eq_ignore_ascii_case("vision") {
                "OpenMindAI Lens is not installed or its vision package is incomplete. Download Lens from Settings > Models first."
            } else {
                "OpenMindAI Core is not installed. Download it from Settings > Models first."
            };
            app_error::AppError::ModelNotFound(message.to_string())
        })?;
''',
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

replace_once(
    "src-tauri/src/lib.rs",
    '''    #[test]
    fn select_conversation_model_routes_vision_to_lens() {
        let core = test_model("core", Some("Qwen/Qwen3-4B-GGUF"), true);
        let lens = test_model("lens", Some("ggml-org/Qwen2.5-VL-3B-Instruct-GGUF"), true);
        let models = vec![core, lens.clone()];

        let selected =
            select_conversation_model(&models, None, "vision", "look at this image").unwrap();

        assert_eq!(selected.id, lens.id);
    }
''',
    '''    #[test]
    fn select_conversation_model_routes_vision_to_lens() {
        let core = test_model("core", Some("Qwen/Qwen3-4B-GGUF"), true);
        let lens = test_model("lens", Some("ggml-org/Qwen2.5-VL-3B-Instruct-GGUF"), true);
        let models = vec![core, lens.clone()];

        let selected =
            select_conversation_model(&models, None, "vision", "look at this image").unwrap();

        assert_eq!(selected.id, lens.id);
    }

    #[test]
    fn select_conversation_model_requires_lens_for_vision() {
        let core = test_model("core", Some("Qwen/Qwen3-4B-GGUF"), true);
        let titan = test_model("titan", Some("Qwen/Qwen3-8B-GGUF"), true);
        let models = vec![core, titan];

        assert!(select_conversation_model(&models, None, "vision", "look at this image").is_none());
    }
''',
)

# Registry: a catalog model with required auxiliary files is only Ready when the
# whole package exists. Also register catalog-declared capabilities for Lens.
replace_once(
    "src-tauri/src/model_registry.rs",
    '''        let existing: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT id FROM model_registry WHERE path = ?1 OR path = ?2",
                params![relative_path, canonical.display().to_string()],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(id) = existing {
            let name = read_model_manifest(&canonical)
                .as_ref()
                .map(display_name_from_manifest);
            self.database.connection().execute(
                "UPDATE model_registry
                 SET path = ?1, name = COALESCE(?2, name), updated_at = ?3
                 WHERE id = ?4",
                params![relative_path, name, Utc::now().to_rfc3339(), id],
            )?;
            return Ok(());
        }

        validate_gguf_header(&canonical, self.root)?;
        let metadata = fs::metadata(&canonical)?;
        let manifest = read_model_manifest(&canonical);
''',
    '''        let manifest = read_model_manifest(&canonical);
        if !catalog_package_ready(&canonical, manifest.as_ref()) {
            self.database.connection().execute(
                "DELETE FROM model_registry WHERE path = ?1 OR path = ?2",
                params![relative_path, canonical.display().to_string()],
            )?;
            tracing::warn!(
                path = %canonical.display(),
                "model package is incomplete; skipping registry activation"
            );
            return Ok(());
        }

        let existing: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT id FROM model_registry WHERE path = ?1 OR path = ?2",
                params![relative_path, canonical.display().to_string()],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(id) = existing {
            let name = manifest.as_ref().map(display_name_from_manifest);
            let capabilities = manifest.as_ref().and_then(catalog_capabilities_from_manifest);
            self.database.connection().execute(
                "UPDATE model_registry
                 SET path = ?1, name = COALESCE(?2, name), capabilities = COALESCE(?3, capabilities), updated_at = ?4
                 WHERE id = ?5",
                params![
                    relative_path,
                    name,
                    capabilities,
                    Utc::now().to_rfc3339(),
                    id
                ],
            )?;
            return Ok(());
        }

        validate_gguf_header(&canonical, self.root)?;
        let metadata = fs::metadata(&canonical)?;
''',
)

replace_once(
    "src-tauri/src/model_registry.rs",
    '''        let capabilities = if manifest
            .as_ref()
            .is_some_and(|manifest| manifest.chat_template_available)
        {
            "[\\"chat\\",\\"thinking\\"]"
        } else {
            "[\\"chat\\"]"
        };
''',
    '''        let capabilities = manifest
            .as_ref()
            .and_then(catalog_capabilities_from_manifest)
            .unwrap_or_else(|| {
                if manifest
                    .as_ref()
                    .is_some_and(|manifest| manifest.chat_template_available)
                {
                    "[\\"chat\\",\\"thinking\\"]".to_string()
                } else {
                    "[\\"chat\\"]".to_string()
                }
            });
''',
)

replace_once(
    "src-tauri/src/model_registry.rs",
    '''fn read_model_manifest(model_path: impl AsRef<Path>) -> Option<QwenModelManifest> {
    let manifest_path = model_path.as_ref().parent()?.join("model-manifest.json");
    let content = fs::read_to_string(manifest_path).ok()?;
    serde_json::from_str(&content).ok()
}
''',
    '''fn read_model_manifest(model_path: impl AsRef<Path>) -> Option<QwenModelManifest> {
    let manifest_path = model_path.as_ref().parent()?.join("model-manifest.json");
    let content = fs::read_to_string(manifest_path).ok()?;
    serde_json::from_str(&content).ok()
}

fn catalog_capabilities_from_manifest(manifest: &QwenModelManifest) -> Option<String> {
    let entry = crate::model_catalog::load_catalog()
        .ok()?
        .models
        .into_iter()
        .find(|entry| entry.repo == manifest.repo)?;
    serde_json::to_string(&entry.capabilities).ok()
}

fn catalog_package_ready(model_path: &Path, manifest: Option<&QwenModelManifest>) -> bool {
    let Some(manifest) = manifest else {
        return true;
    };
    let Ok(catalog) = crate::model_catalog::load_catalog() else {
        return false;
    };
    let Some(entry) = catalog.models.into_iter().find(|entry| entry.repo == manifest.repo) else {
        return true;
    };
    let Some(download) = entry.download else {
        return true;
    };
    let Some(parent) = model_path.parent() else {
        return false;
    };

    download
        .dependencies
        .iter()
        .filter(|dependency| dependency.required)
        .all(|dependency| directory_contains_matching_file(parent, &dependency.filename_pattern))
}

fn directory_contains_matching_file(directory: &Path, pattern: &str) -> bool {
    fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()).map(str::to_string))
        .any(|name| crate::model_catalog::wildcard_match(pattern, &name))
}
''',
)

replace_once(
    "src-tauri/src/model_registry.rs",
    '''    #[test]
    #[ignore = "registers the real downloaded Qwen GGUF in the portable database"]
    fn real_qwen_model_registers_in_portable_database() {
''',
    '''    #[test]
    fn lens_package_requires_mmproj_dependency() {
        let temp = tempfile::tempdir().unwrap();
        let model_dir = temp.path().join("lens");
        fs::create_dir_all(&model_dir).unwrap();
        let model_path = model_dir.join("Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf");
        fs::write(&model_path, b"GGUF").unwrap();
        let manifest = QwenModelManifest {
            repo: "ggml-org/Qwen2.5-VL-3B-Instruct-GGUF".to_string(),
            repo_sha: None,
            quantization: "Q4_K_M".to_string(),
            filename: "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf".to_string(),
            size_bytes: 4,
            sha256: None,
            actual_sha256: None,
            verification: VerificationState::Unverified,
            architecture: Some("qwen2vl".to_string()),
            context_length: Some(8192),
            chat_template_available: true,
            source_url: "https://example.invalid/model.gguf".to_string(),
            installed_at: "now".to_string(),
        };

        assert!(!catalog_package_ready(&model_path, Some(&manifest)));
        fs::write(
            model_dir.join("mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf"),
            b"GGUF",
        )
        .unwrap();
        assert!(catalog_package_ready(&model_path, Some(&manifest)));
        let capabilities = catalog_capabilities_from_manifest(&manifest).unwrap();
        assert!(capabilities.contains("vision"));
        assert!(capabilities.contains("ocr"));
    }

    #[test]
    #[ignore = "registers the real downloaded Qwen GGUF in the portable database"]
    fn real_qwen_model_registers_in_portable_database() {
''',
)

print("vision model integration applied")
