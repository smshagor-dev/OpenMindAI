#[cfg(any(target_os = "android", target_os = "ios"))]
use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[cfg(any(target_os = "android", target_os = "ios"))]
use base64::{engine::general_purpose::STANDARD, Engine as _};
#[cfg(any(target_os = "android", target_os = "ios"))]
use encoding_rs::UTF_8;
#[cfg(any(target_os = "android", target_os = "ios"))]
use llama_cpp_2::{
    context::params::LlamaContextParams,
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{params::LlamaModelParams, AddBos, LlamaChatMessage, LlamaModel},
    mtmd::{mtmd_default_marker, MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputText},
    sampling::LlamaSampler,
};
#[cfg(any(target_os = "android", target_os = "ios"))]
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "android", target_os = "ios"))]
use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};
#[cfg(any(target_os = "android", target_os = "ios"))]
use tauri::Emitter;

use crate::{
    app_error::AppError,
    chat::Message,
    inference::InferenceMedia,
    AppState,
};

#[cfg(any(target_os = "android", target_os = "ios"))]
const MOBILE_VISION_MODEL_ID: &str = "qwen25-vl-3b-q4km";
#[cfg(any(target_os = "android", target_os = "ios"))]
const MOBILE_VISION_CONTEXT_TOKENS: u32 = 4096;
#[cfg(any(target_os = "android", target_os = "ios"))]
const MOBILE_VISION_OUTPUT_TOKENS: u32 = 256;
#[cfg(any(target_os = "android", target_os = "ios"))]
const MOBILE_VISION_BATCH_TOKENS: i32 = 512;
#[cfg(any(target_os = "android", target_os = "ios"))]
const MOBILE_VISION_MAX_IMAGES: usize = 4;
#[cfg(any(target_os = "android", target_os = "ios"))]
const MOBILE_VISION_MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
#[cfg(any(target_os = "android", target_os = "ios"))]
const MOBILE_VISION_MAX_HISTORY_MESSAGES: usize = 16;

#[cfg(any(target_os = "android", target_os = "ios"))]
type StreamCallback<'a> = dyn FnMut(&str) -> Result<(), AppError> + 'a;

#[cfg(any(target_os = "android", target_os = "ios"))]
#[derive(Default)]
struct VisionEngine {
    backend: Option<LlamaBackend>,
    loaded_model: Option<LoadedVisionModel>,
}

#[cfg(any(target_os = "android", target_os = "ios"))]
struct LoadedVisionModel {
    path: PathBuf,
    model: LlamaModel,
}

#[derive(Clone, Default)]
pub(crate) struct MobileVisionState {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    engine: Arc<Mutex<VisionEngine>>,
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[derive(Debug, Clone)]
struct VisionChatMessage {
    role: String,
    content: String,
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[derive(Debug, Clone)]
struct VisionGenerationResult {
    text: String,
    cancelled: bool,
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedMediaRef {
    sha256: String,
    name: String,
    mime_type: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MobileVisionStatus {
    supported: bool,
    installed: bool,
    model_id: String,
    model_name: String,
    reason: String,
}

#[cfg(any(target_os = "android", target_os = "ios"))]
impl VisionEngine {
    fn ensure_model(&mut self, model_path: &Path) -> Result<(), AppError> {
        if self.backend.is_none() {
            self.backend = Some(
                LlamaBackend::init()
                    .map_err(|error| AppError::ModelLoadFailed(error.to_string()))?,
            );
        }
        if self
            .loaded_model
            .as_ref()
            .is_some_and(|loaded| loaded.path == model_path)
        {
            return Ok(());
        }

        let backend = self.backend.as_ref().ok_or_else(|| {
            AppError::ModelLoadFailed("mobile vision llama backend unavailable".to_string())
        })?;
        self.loaded_model = None;
        let model = LlamaModel::load_from_file(backend, model_path, &LlamaModelParams::default())
            .map_err(|error| AppError::ModelLoadFailed(error.to_string()))?;
        self.loaded_model = Some(LoadedVisionModel {
            path: model_path.to_path_buf(),
            model,
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn generate(
        &mut self,
        model_path: &Path,
        mmproj_path: &Path,
        messages: Vec<VisionChatMessage>,
        media: Vec<InferenceMedia>,
        cancellation: &tokio_util::sync::CancellationToken,
        mut on_chunk: Option<&mut StreamCallback<'_>>,
    ) -> Result<VisionGenerationResult, AppError> {
        self.ensure_model(model_path)?;
        let backend = self.backend.as_ref().ok_or_else(|| {
            AppError::ModelLoadFailed("mobile vision llama backend unavailable".to_string())
        })?;
        let model = &self
            .loaded_model
            .as_ref()
            .ok_or_else(|| AppError::ModelLoadFailed("mobile vision model is not loaded".to_string()))?
            .model;

        let mut mtmd_params = MtmdContextParams::default();
        mtmd_params.use_gpu = cfg!(target_os = "ios");
        mtmd_params.print_timings = false;
        mtmd_params.n_threads = 4;
        mtmd_params.image_min_tokens = 96;
        mtmd_params.image_max_tokens = 512;
        let mmproj = mmproj_path.to_str().ok_or_else(|| {
            AppError::ModelUnsupported("mobile vision mmproj path is not valid UTF-8".to_string())
        })?;
        let mtmd = MtmdContext::init_from_file(mmproj, model, &mtmd_params).map_err(|error| {
            AppError::ModelLoadFailed(format!("failed to initialize multimodal projector: {error}"))
        })?;
        if !mtmd.support_vision() {
            return Err(AppError::ModelUnsupported(
                "installed OpenMindAI Lens package does not expose vision input".to_string(),
            ));
        }

        if media.is_empty() || media.len() > MOBILE_VISION_MAX_IMAGES {
            return Err(AppError::InferenceFailed(format!(
                "mobile vision requires 1-{MOBILE_VISION_MAX_IMAGES} images"
            )));
        }
        let prompt = build_vision_prompt(model, messages, media.len())?;
        let bitmaps = media
            .iter()
            .map(|item| {
                let bytes = decode_image(item)?;
                MtmdBitmap::from_buffer(&mtmd, &bytes, false).map_err(|error| {
                    AppError::InferenceFailed(format!("failed to decode vision image: {error}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let bitmap_refs = bitmaps.iter().collect::<Vec<_>>();
        let chunks = mtmd
            .tokenize(
                MtmdInputText {
                    text: prompt,
                    add_special: true,
                    parse_special: true,
                },
                &bitmap_refs,
            )
            .map_err(|error| {
                AppError::InferenceFailed(format!("failed to tokenize multimodal input: {error}"))
            })?;

        let required_tokens = chunks
            .total_tokens()
            .saturating_add(MOBILE_VISION_OUTPUT_TOKENS as usize);
        if required_tokens > MOBILE_VISION_CONTEXT_TOKENS as usize {
            return Err(AppError::ContextOverflow(format!(
                "mobile visual context requires {required_tokens} tokens; reduce the number/detail of images and retry"
            )));
        }

        let context_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(MOBILE_VISION_CONTEXT_TOKENS))
            .with_n_batch(MOBILE_VISION_BATCH_TOKENS as u32);
        let mut context = model
            .new_context(backend, context_params)
            .map_err(|error| AppError::ModelLoadFailed(format!("vision context failed: {error}")))?;
        let mut position = chunks
            .eval_chunks(&mtmd, &context, 0, 0, MOBILE_VISION_BATCH_TOKENS, true)
            .map_err(|error| {
                AppError::InferenceFailed(format!("multimodal prompt evaluation failed: {error}"))
            })?;

        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_k(20),
            LlamaSampler::top_p(0.95, 1),
            LlamaSampler::temp(0.4),
            LlamaSampler::dist(0x5649_534e),
        ]);
        let mut decoder = UTF_8.new_decoder();
        let mut output = String::new();
        let mut cancelled = false;
        let mut batch = LlamaBatch::new(1, 1);

        for _ in 0..MOBILE_VISION_OUTPUT_TOKENS {
            if cancellation.is_cancelled() {
                cancelled = true;
                break;
            }
            let token = sampler.sample(&context, -1);
            sampler.accept(token);
            if model.is_eog_token(token) {
                break;
            }
            let piece = model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|error| {
                    AppError::InferenceFailed(format!("failed to decode vision token: {error}"))
                })?;
            if !piece.is_empty() {
                output.push_str(&piece);
                if let Some(callback) = on_chunk.as_deref_mut() {
                    callback(&piece)?;
                }
            }

            batch.clear();
            batch.add(token, position, &[0], true).map_err(|error| {
                AppError::InferenceFailed(format!("failed to build vision decode batch: {error}"))
            })?;
            context.decode(&mut batch).map_err(|error| {
                AppError::InferenceFailed(format!("vision token decode failed: {error}"))
            })?;
            position += 1;
        }

        Ok(VisionGenerationResult {
            text: output,
            cancelled,
        })
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn build_vision_prompt(
    model: &LlamaModel,
    messages: Vec<VisionChatMessage>,
    media_count: usize,
) -> Result<String, AppError> {
    let mut normalized = messages
        .into_iter()
        .filter_map(|message| {
            let role = message.role.trim().to_ascii_lowercase();
            let content = message.content.trim().to_string();
            (!content.is_empty() && matches!(role.as_str(), "system" | "user" | "assistant"))
                .then_some(VisionChatMessage { role, content })
        })
        .collect::<Vec<_>>();
    if normalized.len() > MOBILE_VISION_MAX_HISTORY_MESSAGES {
        let system = normalized
            .iter()
            .filter(|message| message.role == "system")
            .cloned()
            .collect::<Vec<_>>();
        let mut conversational = normalized
            .into_iter()
            .filter(|message| message.role != "system")
            .collect::<Vec<_>>();
        let keep = MOBILE_VISION_MAX_HISTORY_MESSAGES.saturating_sub(system.len());
        if conversational.len() > keep {
            conversational.drain(..conversational.len() - keep);
        }
        normalized = system;
        normalized.extend(conversational);
    }

    let latest_user = normalized
        .iter_mut()
        .rfind(|message| message.role == "user")
        .ok_or_else(|| AppError::InferenceFailed("vision request has no user message".to_string()))?;
    let markers = (0..media_count)
        .map(|index| format!("Image {}: {}", index + 1, mtmd_default_marker()))
        .collect::<Vec<_>>()
        .join("\n");
    latest_user.content = format!("{}\n\n{}", latest_user.content.trim(), markers);

    let template = model.chat_template(None).map_err(|error| {
        AppError::ModelUnsupported(format!("vision model chat template unavailable: {error}"))
    })?;
    let encoded = normalized
        .into_iter()
        .map(|message| LlamaChatMessage::new(message.role, message.content))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::InferenceFailed(format!("vision chat encoding failed: {error}")))?;
    model
        .apply_chat_template(&template, &encoded, true)
        .map_err(|error| AppError::InferenceFailed(format!("vision chat template failed: {error}")))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn decode_image(item: &InferenceMedia) -> Result<Vec<u8>, AppError> {
    if item.kind != "image" || !matches!(item.mime_type.as_str(), "image/png" | "image/jpeg") {
        return Err(AppError::InferenceFailed(
            "mobile vision accepts optimized PNG or JPEG images only".to_string(),
        ));
    }
    let prefix = format!("data:{};base64,", item.mime_type);
    let encoded = item.data_url.strip_prefix(&prefix).ok_or_else(|| {
        AppError::InferenceFailed("vision image data URL does not match its MIME type".to_string())
    })?;
    let bytes = STANDARD.decode(encoded).map_err(|error| {
        AppError::InferenceFailed(format!("vision image base64 is invalid: {error}"))
    })?;
    if bytes.is_empty() || bytes.len() > MOBILE_VISION_MAX_IMAGE_BYTES {
        return Err(AppError::InferenceFailed(format!(
            "each optimized mobile vision image must be at most {} MB",
            MOBILE_VISION_MAX_IMAGE_BYTES / (1024 * 1024)
        )));
    }
    validate_signature(&bytes, &item.mime_type)?;
    Ok(bytes)
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn validate_signature(bytes: &[u8], mime_type: &str) -> Result<(), AppError> {
    let valid = match mime_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(AppError::InferenceFailed(
            "vision image bytes do not match the declared MIME type".to_string(),
        ))
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn vision_model_paths(state: &AppState) -> Result<(PathBuf, PathBuf, String), AppError> {
    let entry = crate::model_catalog::entry_by_id(MOBILE_VISION_MODEL_ID)?;
    let status = crate::installed_catalog_entry_by_id(state, MOBILE_VISION_MODEL_ID)?.ok_or_else(|| {
        AppError::ModelNotFound(
            "OpenMindAI Lens is not installed. Open Models and download OpenMindAI Lens for local image/OCR analysis."
                .to_string(),
        )
    })?;
    let primary = status.installed_path.ok_or_else(|| {
        AppError::ModelNotFound("OpenMindAI Lens model path is unavailable".to_string())
    })?;
    let model_path = state.root.resolve_relative(&primary)?;
    let download = entry.download.as_ref().ok_or_else(|| {
        AppError::ModelUnsupported("OpenMindAI Lens download package is not configured".to_string())
    })?;
    let mmproj_pattern = download
        .dependencies
        .iter()
        .find(|dependency| dependency.role == "mmproj" && dependency.required)
        .map(|dependency| dependency.filename_pattern.as_str())
        .ok_or_else(|| AppError::ModelUnsupported("OpenMindAI Lens mmproj is not configured".to_string()))?;
    let mmproj_path = crate::model_catalog::installed_file_for_pattern(
        &state.root,
        &download.destination_dir,
        mmproj_pattern,
    )
    .ok_or_else(|| {
        AppError::ModelNotFound(
            "OpenMindAI Lens multimodal projector is missing. Re-download the Lens package."
                .to_string(),
        )
    })?;
    Ok((model_path, mmproj_path, entry.name))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn persist_media(
    state: &AppState,
    message_id: &str,
    media: &[InferenceMedia],
) -> Result<(), AppError> {
    if media.is_empty() || media.len() > MOBILE_VISION_MAX_IMAGES {
        return Err(AppError::InferenceFailed(format!(
            "mobile vision accepts 1-{MOBILE_VISION_MAX_IMAGES} images per request"
        )));
    }
    let media_dir = state.root.resolve_relative("data/media")?;
    let index_dir = state.root.resolve_relative("data/media-index")?;
    fs::create_dir_all(&media_dir)?;
    fs::create_dir_all(&index_dir)?;
    let mut refs = Vec::with_capacity(media.len());

    for item in media {
        let bytes = decode_image(item)?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let extension = if item.mime_type == "image/png" { "png" } else { "jpg" };
        let path = media_dir.join(format!("{sha256}.{extension}"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(&bytes)?;
                file.flush()?;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(AppError::from(error)),
        }
        refs.push(PersistedMediaRef {
            sha256,
            name: item.name.chars().take(240).collect(),
            mime_type: item.mime_type.clone(),
        });
    }
    fs::write(
        index_dir.join(format!("{message_id}.json")),
        serde_json::to_vec(&refs).map_err(|error| AppError::internal(error.to_string()))?,
    )?;
    Ok(())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn load_media(state: &AppState, message_id: &str) -> Result<Vec<InferenceMedia>, AppError> {
    let index = state
        .root
        .resolve_relative("data/media-index")?
        .join(format!("{message_id}.json"));
    if !index.is_file() {
        return Ok(Vec::new());
    }
    let refs: Vec<PersistedMediaRef> = serde_json::from_slice(&fs::read(index)?)
        .map_err(|error| AppError::InferenceFailed(format!("stored vision index is invalid: {error}")))?;
    let media_dir = state.root.resolve_relative("data/media")?;
    refs.into_iter()
        .take(MOBILE_VISION_MAX_IMAGES)
        .map(|item| {
            if item.sha256.len() != 64 || !item.sha256.chars().all(|value| value.is_ascii_hexdigit()) {
                return Err(AppError::InferenceFailed(
                    "stored visual evidence failed integrity validation".to_string(),
                ));
            }
            let extension = match item.mime_type.as_str() {
                "image/png" => "png",
                "image/jpeg" => "jpg",
                _ => return Err(AppError::InferenceFailed("stored vision MIME type is unsupported".to_string())),
            };
            let bytes = fs::read(media_dir.join(format!("{}.{}", item.sha256, extension)))?;
            validate_signature(&bytes, &item.mime_type)?;
            let actual = format!("{:x}", Sha256::digest(&bytes));
            if actual != item.sha256 {
                return Err(AppError::InferenceFailed(
                    "stored visual evidence failed checksum validation".to_string(),
                ));
            }
            Ok(InferenceMedia {
                kind: "image".to_string(),
                name: item.name,
                mime_type: item.mime_type.clone(),
                data_url: format!("data:{};base64,{}", item.mime_type, STANDARD.encode(bytes)),
            })
        })
        .collect()
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn history_for_vision(
    state: &AppState,
    conversation_id: &str,
    assistant_id: &str,
) -> Result<Vec<VisionChatMessage>, AppError> {
    use crate::chat::ChatRepository;
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    Ok(ChatRepository::new(&db)
        .list_messages(conversation_id)?
        .into_iter()
        .filter(|message| {
            message.id != assistant_id
                && (message.role == "system" || message.status == "completed")
        })
        .map(|message| VisionChatMessage {
            role: message.role,
            content: message.content,
        })
        .collect())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[allow(clippy::too_many_arguments)]
async fn run_vision_generation(
    app: &AppHandle,
    state: &State<'_, AppState>,
    native: MobileVisionState,
    conversation_id: &str,
    assistant: &Message,
    model_path: PathBuf,
    mmproj_path: PathBuf,
    media: Vec<InferenceMedia>,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<Message, AppError> {
    use crate::{
        chat::ChatRepository,
        inference::{StreamChunkEvent, StreamDoneEvent},
    };
    let history = history_for_vision(state, conversation_id, &assistant.id)?;
    let stream_app = app.clone();
    let stream_conversation_id = conversation_id.to_string();
    let stream_message_id = assistant.id.clone();
    let engine = native.engine.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut engine = engine
            .lock()
            .map_err(|_| AppError::internal("mobile vision inference lock poisoned"))?;
        engine.generate(
            &model_path,
            &mmproj_path,
            history,
            media,
            &cancellation,
            Some(&mut |chunk| {
                stream_app
                    .emit(
                        "inference:chunk",
                        StreamChunkEvent {
                            conversation_id: stream_conversation_id.clone(),
                            message_id: stream_message_id.clone(),
                            chunk: chunk.to_string(),
                        },
                    )
                    .map_err(|error| AppError::StreamFailed(error.to_string()))
            }),
        )
    })
    .await
    .map_err(|error| AppError::InferenceFailed(format!("vision worker failed: {error}")))?;

    let generation = match result {
        Ok(result) => result,
        Err(error) => {
            mark_failed(state, conversation_id, &assistant.id, app);
            return Err(error);
        }
    };
    let status = if generation.cancelled { "cancelled" } else { "completed" };
    let completed = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        let repo = ChatRepository::new(&db);
        if !generation.text.is_empty() {
            repo.append_message_chunk(&assistant.id, &generation.text)?;
        }
        repo.set_message_status(&assistant.id, status)?;
        repo.list_messages(conversation_id)?
            .into_iter()
            .find(|message| message.id == assistant.id)
            .ok_or_else(|| AppError::internal("completed mobile vision response disappeared"))?
    };
    state.active_generations.finish(conversation_id);
    app.emit(
        "inference:done",
        StreamDoneEvent {
            conversation_id: conversation_id.to_string(),
            message_id: assistant.id.clone(),
            status: status.to_string(),
        },
    )
    .map_err(|error| AppError::StreamFailed(error.to_string()))?;
    Ok(completed)
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn mark_failed(
    state: &State<'_, AppState>,
    conversation_id: &str,
    assistant_message_id: &str,
    app: &AppHandle,
) {
    use crate::{chat::ChatRepository, inference::StreamDoneEvent};
    if let Ok(db) = state.database.lock() {
        let _ = ChatRepository::new(&db).set_message_status(assistant_message_id, "failed");
    }
    state.active_generations.finish(conversation_id);
    let _ = app.emit(
        "inference:done",
        StreamDoneEvent {
            conversation_id: conversation_id.to_string(),
            message_id: assistant_message_id.to_string(),
            status: "failed".to_string(),
        },
    );
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub(crate) fn mobile_vision_status(state: State<'_, AppState>) -> MobileVisionStatus {
    match vision_model_paths(&state) {
        Ok(_) => MobileVisionStatus {
            supported: true,
            installed: true,
            model_id: MOBILE_VISION_MODEL_ID.to_string(),
            model_name: "OpenMindAI Lens".to_string(),
            reason: "Native llama.cpp MTMD vision/OCR is ready.".to_string(),
        },
        Err(error) => MobileVisionStatus {
            supported: true,
            installed: false,
            model_id: MOBILE_VISION_MODEL_ID.to_string(),
            model_name: "OpenMindAI Lens".to_string(),
            reason: error.to_string(),
        },
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub(crate) fn mobile_vision_status(_state: State<'_, AppState>) -> MobileVisionStatus {
    MobileVisionStatus {
        supported: false,
        installed: false,
        model_id: "qwen25-vl-3b-q4km".to_string(),
        model_name: "OpenMindAI Lens".to_string(),
        reason: "Mobile vision status is only available in Android/iOS builds.".to_string(),
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub(crate) async fn mobile_send_vision_message(
    app: AppHandle,
    conversation_id: String,
    content: String,
    media: Vec<InferenceMedia>,
    state: State<'_, AppState>,
    native: State<'_, MobileVisionState>,
) -> Result<Message, AppError> {
    use crate::{chat::ChatRepository, inference::StreamStartedEvent};
    let prompt = if content.trim().is_empty() {
        "Analyze the attached visual evidence carefully. Extract text when relevant and answer accurately."
            .to_string()
    } else {
        content.trim().to_string()
    };
    let (model_path, mmproj_path, model_name) = vision_model_paths(&state)?;
    let cancellation = state.active_generations.start(&conversation_id)?;
    crate::sync_project_context(&state, &conversation_id)?;

    let created = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        let repo = ChatRepository::new(&db);
        let user = repo.add_message(
            &conversation_id,
            "user",
            &prompt,
            "completed",
            Some(MOBILE_VISION_MODEL_ID),
        )?;
        let assistant = repo.add_message(
            &conversation_id,
            "assistant",
            "",
            "streaming",
            Some(MOBILE_VISION_MODEL_ID),
        )?;
        (user, assistant)
    };
    let (user, assistant) = created;
    if let Err(error) = persist_media(&state, &user.id, &media) {
        if let Ok(db) = state.database.lock() {
            let repo = ChatRepository::new(&db);
            let _ = repo.delete_message(&assistant.id);
            let _ = repo.delete_message(&user.id);
        }
        state.active_generations.finish(&conversation_id);
        return Err(error);
    }
    if let Err(error) = app.emit(
        "inference:started",
        StreamStartedEvent {
            conversation_id: conversation_id.clone(),
            user,
            assistant: assistant.clone(),
            routed_model_name: model_name,
            routing_reason: "Task router selected OpenMindAI Lens because visual media is attached."
                .to_string(),
        },
    ) {
        mark_failed(&state, &conversation_id, &assistant.id, &app);
        return Err(AppError::StreamFailed(error.to_string()));
    }

    run_vision_generation(
        &app,
        &state,
        native.inner().clone(),
        &conversation_id,
        &assistant,
        model_path,
        mmproj_path,
        media,
        cancellation,
    )
    .await
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub(crate) async fn mobile_send_vision_message(
    _app: AppHandle,
    _conversation_id: String,
    _content: String,
    _media: Vec<InferenceMedia>,
    _state: State<'_, AppState>,
) -> Result<Message, AppError> {
    Err(AppError::ModelUnsupported(
        "native mobile vision is only available in Android/iOS builds".to_string(),
    ))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub(crate) async fn mobile_regenerate_vision_message(
    app: AppHandle,
    conversation_id: String,
    assistant_message_id: String,
    state: State<'_, AppState>,
    native: State<'_, MobileVisionState>,
) -> Result<Message, AppError> {
    use crate::{chat::ChatRepository, inference::StreamStartedEvent};
    let (model_path, mmproj_path, model_name) = vision_model_paths(&state)?;
    let cancellation = state.active_generations.start(&conversation_id)?;
    let (user, assistant, media) = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        let repo = ChatRepository::new(&db);
        let history = repo.list_messages(&conversation_id)?;
        let target_index = history
            .iter()
            .position(|message| message.id == assistant_message_id)
            .ok_or_else(|| AppError::internal("assistant message not found"))?;
        let user = history[..target_index]
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .cloned()
            .ok_or_else(|| AppError::internal("no preceding visual user message"))?;
        let media = load_media(&state, &user.id)?;
        if media.is_empty() {
            state.active_generations.finish(&conversation_id);
            return Err(AppError::InferenceFailed(
                "The original visual evidence is unavailable. Reattach it and send again."
                    .to_string(),
            ));
        }
        repo.delete_message(&assistant_message_id)?;
        let assistant = repo.add_message(
            &conversation_id,
            "assistant",
            "",
            "streaming",
            Some(MOBILE_VISION_MODEL_ID),
        )?;
        (user, assistant, media)
    };
    app.emit(
        "inference:started",
        StreamStartedEvent {
            conversation_id: conversation_id.clone(),
            user,
            assistant: assistant.clone(),
            routed_model_name: model_name,
            routing_reason: "Regenerating with the persisted OpenMindAI Lens visual evidence."
                .to_string(),
        },
    )
    .map_err(|error| AppError::StreamFailed(error.to_string()))?;

    run_vision_generation(
        &app,
        &state,
        native.inner().clone(),
        &conversation_id,
        &assistant,
        model_path,
        mmproj_path,
        media,
        cancellation,
    )
    .await
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub(crate) async fn mobile_regenerate_vision_message(
    _app: AppHandle,
    _conversation_id: String,
    _assistant_message_id: String,
    _state: State<'_, AppState>,
) -> Result<Message, AppError> {
    Err(AppError::ModelUnsupported(
        "native mobile vision regeneration is only available in Android/iOS builds".to_string(),
    ))
}
