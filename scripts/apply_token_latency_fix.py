from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, found {count}\n--- expected ---\n{old}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


def replace_all_exact(path: str, old: str, new: str, expected: int) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    count = text.count(old)
    if count != expected:
        raise SystemExit(f"{path}: expected {expected} matches, found {count}\n--- expected ---\n{old}")
    file.write_text(text.replace(old, new), encoding="utf-8")


# P0: use physical cores for llama.cpp worker threads and enable flash attention
# only on backends where this project can safely opt into it today.
replace_once(
    "src-tauri/src/performance.rs",
    '''        PerformanceProfile {
            mode: PerformanceMode::Auto,
            recommended_backend,
            cpu_threads: hardware.cpu.logical_threads.saturating_sub(2).max(1),
            system_memory_budget_bytes,
            vram_budget_bytes,
            mmap: true,
            flash_attention: false,
        }
''',
    '''        let cpu_threads = hardware
            .cpu
            .physical_cores
            .unwrap_or_else(|| hardware.cpu.logical_threads.saturating_sub(2).max(1))
            .min(hardware.cpu.logical_threads.max(1))
            .max(1);
        let flash_attention = matches!(
            &recommended_backend,
            BackendKind::Cuda | BackendKind::Hip | BackendKind::Sycl | BackendKind::Metal
        );

        PerformanceProfile {
            mode: PerformanceMode::Auto,
            recommended_backend,
            cpu_threads,
            system_memory_budget_bytes,
            vram_budget_bytes,
            mmap: true,
            flash_attention,
        }
''',
)
replace_once(
    "src-tauri/src/performance.rs",
    '''        assert!(profile.vram_budget_bytes.unwrap() < 8 * 1024 * 1024 * 1024);
''',
    '''        assert!(profile.vram_budget_bytes.unwrap() < 8 * 1024 * 1024 * 1024);
        assert_eq!(profile.cpu_threads, 6);
        assert!(profile.flash_attention);
''',
)

# P0: correct the KV-cache estimate so an 8 GB card does not get needlessly
# pushed into partial offload for Qwen3 4B, support HIP/SYCL/Metal placement,
# increase prompt micro-batch size, and honor the capability-gated FA profile.
replace_once(
    "src-tauri/src/launch_planner.rs",
    '''        let gpu_layers = match (backend.clone(), dedicated_vram_budget_bytes) {
            (BackendKind::Cuda, Some(budget)) if budget > total_estimate => 999,
            (BackendKind::Cuda, Some(budget)) if budget > estimated_model_bytes / 2 => 32,
            (BackendKind::Cuda, Some(_)) => 16,
            (BackendKind::Vulkan, Some(budget)) if budget > total_estimate => 999,
            (BackendKind::Vulkan, Some(budget)) if budget > estimated_model_bytes / 2 => 24,
            (BackendKind::Vulkan, Some(_)) => 12,
            _ => 0,
        };
''',
    '''        let gpu_layers = match (backend.clone(), dedicated_vram_budget_bytes) {
            (BackendKind::Cuda | BackendKind::Hip | BackendKind::Metal, Some(budget))
                if budget > total_estimate =>
            {
                999
            }
            (BackendKind::Cuda | BackendKind::Hip | BackendKind::Metal, Some(budget))
                if budget > estimated_model_bytes / 2 =>
            {
                32
            }
            (BackendKind::Cuda | BackendKind::Hip | BackendKind::Metal, Some(_)) => 16,
            (BackendKind::Vulkan | BackendKind::Sycl, Some(budget))
                if budget > total_estimate =>
            {
                999
            }
            (BackendKind::Vulkan | BackendKind::Sycl, Some(budget))
                if budget > estimated_model_bytes / 2 =>
            {
                24
            }
            (BackendKind::Vulkan | BackendKind::Sycl, Some(_)) => 12,
            _ => 0,
        };
''',
)
replace_once(
    "src-tauri/src/launch_planner.rs",
    '''                batch_size: 512,
                ubatch_size: 128,
                flash_attention: false,
''',
    '''                batch_size: 512,
                ubatch_size: 256,
                flash_attention: profile.flash_attention,
''',
)
replace_once(
    "src-tauri/src/launch_planner.rs",
    '''fn estimate_context_bytes(context_size: u32) -> u64 {
    u64::from(context_size) * 1024 * 1024 / 2
}
''',
    '''fn estimate_context_bytes(context_size: u32) -> u64 {
    // Qwen-class GQA models use far less KV memory than the previous generic
    // 512 KiB/token estimate. 192 KiB/token keeps a safety margin while
    // avoiding false partial-offload decisions on common 8 GB GPUs.
    const KV_BYTES_PER_TOKEN_ESTIMATE: u64 = 192 * 1024;
    u64::from(context_size) * KV_BYTES_PER_TOKEN_ESTIMATE
}
''',
)
replace_once(
    "src-tauri/src/launch_planner.rs",
    '''        assert!(plan.dedicated_vram_budget_bytes.unwrap() < 8 * 1024 * 1024 * 1024);
        assert_ne!(
''',
    '''        assert!(plan.dedicated_vram_budget_bytes.unwrap() < 8 * 1024 * 1024 * 1024);
        assert_eq!(plan.config.gpu_layers, 999);
        assert_eq!(plan.estimated_context_bytes, 8192 * 192 * 1024);
        assert_ne!(
''',
)

# P0/P2: tell llama-server to use the same tuned thread count for prompt
# batches as token decoding.
replace_once(
    "src-tauri/src/runtime.rs",
    '''            "--threads".to_string(),
            config.threads.to_string(),
            "--batch-size".to_string(),
''',
    '''            "--threads".to_string(),
            config.threads.to_string(),
            "--threads-batch".to_string(),
            config.threads.to_string(),
            "--batch-size".to_string(),
''',
)

# P3: batch UI stream events and SQLite writes, shorten transient single-slot
# retries, and send the actual routed model id instead of a hard-coded Core id.
replace_once(
    "src-tauri/src/inference.rs",
    '''const MAX_INFERENCE_MEDIA_ITEMS: usize = 4;
''',
    '''const MAX_INFERENCE_MEDIA_ITEMS: usize = 4;
const UI_STREAM_CHUNK_BYTES: usize = 32;
const DB_STREAM_FLUSH_BYTES: usize = 2_048;
''',
)
replace_once(
    "src-tauri/src/inference.rs",
    '''    pub endpoint: &'a str,
    pub conversation_id: &'a str,
''',
    '''    pub endpoint: &'a str,
    pub model: &'a str,
    pub conversation_id: &'a str,
''',
)
replace_once(
    "src-tauri/src/inference.rs",
    '''        "model": "qwen3-4b-q4_k_m",
''',
    '''        "model": request.model,
''',
)
replace_once(
    "src-tauri/src/inference.rs",
    '''    let mut sse_buffer = String::new();
    let mut flush_buffer = String::new();
    let mut generated_chars = 0;
''',
    '''    let mut sse_buffer = String::new();
    let mut flush_buffer = String::new();
    let mut ui_buffer = String::new();
    let mut generated_chars = 0;
''',
)
replace_once(
    "src-tauri/src/inference.rs",
    '''                    generated_chars += token.chars().count();
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
''',
    '''                    generated_chars += token.chars().count();
                    flush_buffer.push_str(&token);
                    ui_buffer.push_str(&token);

                    if ui_buffer.len() >= UI_STREAM_CHUNK_BYTES {
                        emit_stream_chunk(&request, &mut ui_buffer)?;
                    }
                    if flush_buffer.len() >= DB_STREAM_FLUSH_BYTES {
                        flush(request.database, &request.assistant.id, &mut flush_buffer)?;
                    }
''',
)
replace_once(
    "src-tauri/src/inference.rs",
    '''    if !flush_buffer.is_empty() {
        flush(request.database, &request.assistant.id, &mut flush_buffer)?;
    }
''',
    '''    if !ui_buffer.is_empty() {
        emit_stream_chunk(&request, &mut ui_buffer)?;
    }
    if !flush_buffer.is_empty() {
        flush(request.database, &request.assistant.id, &mut flush_buffer)?;
    }
''',
)
replace_once(
    "src-tauri/src/inference.rs",
    '''    Ok(InferenceMetrics {
        time_to_first_token_ms: first_token_at,
        generated_chars,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn attach_media_to_latest_user_message(
''',
    '''    Ok(InferenceMetrics {
        time_to_first_token_ms: first_token_at,
        generated_chars,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn emit_stream_chunk(request: &StreamRequest<'_>, buffer: &mut String) -> Result<(), AppError> {
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

fn attach_media_to_latest_user_message(
''',
)
replace_once(
    "src-tauri/src/inference.rs",
    '''const COMPLETION_RETRY_ATTEMPTS: u32 = 5;
const COMPLETION_RETRY_DELAY_MS: u64 = 600;
''',
    '''const COMPLETION_RETRY_ATTEMPTS: u32 = 3;
const COMPLETION_RETRY_DELAY_MS: u64 = 200;
''',
)

# P0: transient modes must not permanently switch a conversation to 8B.
# Search and ordinary coding stay on the selected/Core model; explicit
# Thinking and Deep Research can still route to 8B.
replace_once(
    "src-tauri/src/lib_legacy.rs",
    '''    if active_model_id.as_deref() != Some(selected.id.as_str()) {
        repo.set_active_model(conversation_id, Some(&selected.id))?;
    }
''',
    '''    if mode.eq_ignore_ascii_case("chat")
        && active_model_id.as_deref() != Some(selected.id.as_str())
    {
        repo.set_active_model(conversation_id, Some(&selected.id))?;
    }
''',
)
replace_once(
    "src-tauri/src/lib_legacy.rs",
    '''        endpoint: &endpoint,
        conversation_id,
''',
    '''        endpoint: &endpoint,
        model: &model.id,
        conversation_id,
''',
)
replace_once(
    "src-tauri/src/lib_legacy.rs",
    '''    let result = inference::stream_chat_completion(StreamRequest {
        app,
        database: &state.database,
        active: &state.active_generations,
        client: &state.http,
        endpoint: &endpoint,
        model: &model.id,
        conversation_id,
        assistant,
        mode: inference_mode,
        media,
    })
    .await;

    if let Err(error) = result {
''',
    '''    let result = inference::stream_chat_completion(StreamRequest {
        app,
        database: &state.database,
        active: &state.active_generations,
        client: &state.http,
        endpoint: &endpoint,
        model: &model.id,
        conversation_id,
        assistant,
        mode: inference_mode,
        media,
    })
    .await;

    if let Ok(metrics) = &result {
        tracing::info!(
            model = %model.name,
            mode,
            time_to_first_token_ms = ?metrics.time_to_first_token_ms,
            generated_chars = metrics.generated_chars,
            elapsed_ms = metrics.elapsed_ms,
            "local inference completed"
        );
    }

    if let Err(error) = result {
''',
)
replace_once(
    "src-tauri/src/lib_legacy.rs",
    '''    let normalized_mode = mode.to_ascii_lowercase();
    let normalized_content = content.to_ascii_lowercase();

    if normalized_mode == "vision" {
''',
    '''    let normalized_mode = mode.to_ascii_lowercase();

    if normalized_mode == "vision" {
''',
)
replace_once(
    "src-tauri/src/lib_legacy.rs",
    '''    if matches!(
        normalized_mode.as_str(),
        "thinking" | "research" | "search" | "image" | "video" | "voice"
    ) || looks_like_code_task(&normalized_content)
    {
''',
    '''    if matches!(
        normalized_mode.as_str(),
        "thinking" | "research" | "image" | "video" | "voice"
    ) {
''',
)

# P1: cache expensive recursive local-workspace context scans for 30 seconds.
# Mutations invalidate immediately; external editor changes are picked up when
# the short TTL expires.
replace_once(
    "src-tauri/src/local_workspace.rs",
    '''use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};
''',
    '''use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};
''',
)
replace_once(
    "src-tauri/src/local_workspace.rs",
    '''const WORKSPACE_CONTEXT_TOTAL_CHARS: usize = 7_000;
''',
    '''const WORKSPACE_CONTEXT_TOTAL_CHARS: usize = 7_000;
const WORKSPACE_CONTEXT_CACHE_TTL_SECS: u64 = 30;

#[derive(Clone)]
struct CachedWorkspaceContext {
    created_at: Instant,
    value: Option<String>,
}

static WORKSPACE_CONTEXT_CACHE: OnceLock<Mutex<HashMap<String, CachedWorkspaceContext>>> =
    OnceLock::new();

fn cached_workspace_context(project_id: &str) -> Option<Option<String>> {
    let cache = WORKSPACE_CONTEXT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().ok()?;
    let cached = cache.get(project_id)?.clone();
    if cached.created_at.elapsed() > Duration::from_secs(WORKSPACE_CONTEXT_CACHE_TTL_SECS) {
        cache.remove(project_id);
        return None;
    }
    Some(cached.value)
}

fn store_workspace_context(project_id: &str, value: Option<String>) {
    let cache = WORKSPACE_CONTEXT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut cache) = cache.lock() {
        cache.insert(
            project_id.to_string(),
            CachedWorkspaceContext {
                created_at: Instant::now(),
                value,
            },
        );
    }
}

pub(crate) fn invalidate_project_workspace_context(project_id: &str) {
    let cache = WORKSPACE_CONTEXT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut cache) = cache.lock() {
        cache.remove(project_id);
    }
}
''',
)
replace_once(
    "src-tauri/src/local_workspace.rs",
    '''pub(crate) fn workspace_context_for_project(
    database: &Database,
    project_id: &str,
) -> Result<Option<String>, AppError> {
    let config = load_config(database, project_id)?;
    if config.roots.is_empty() {
        return Ok(None);
    }
''',
    '''pub(crate) fn workspace_context_for_project(
    database: &Database,
    project_id: &str,
) -> Result<Option<String>, AppError> {
    if let Some(cached) = cached_workspace_context(project_id) {
        return Ok(cached);
    }

    let config = load_config(database, project_id)?;
    if config.roots.is_empty() {
        store_workspace_context(project_id, None);
        return Ok(None);
    }
''',
)
replace_once(
    "src-tauri/src/local_workspace.rs",
    '''    Ok(Some(sections.join("\\n\\n")))
}

pub(crate) fn clear_project_workspace_config(
''',
    '''    let context = Some(sections.join("\\n\\n"));
    store_workspace_context(project_id, context.clone());
    Ok(context)
}

pub(crate) fn clear_project_workspace_config(
''',
)
replace_once(
    "src-tauri/src/local_workspace.rs",
    '''    database.connection().execute(
        "DELETE FROM app_settings WHERE key = ?1",
        params![config_key(project_id)],
    )?;
    Ok(())
}
''',
    '''    database.connection().execute(
        "DELETE FROM app_settings WHERE key = ?1",
        params![config_key(project_id)],
    )?;
    invalidate_project_workspace_context(project_id);
    Ok(())
}
''',
)
replace_once(
    "src-tauri/src/local_workspace.rs",
    '''    database.connection().execute(
        "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
        params![config_key(project_id), value, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}
''',
    '''    database.connection().execute(
        "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
        params![config_key(project_id), value, Utc::now().to_rfc3339()],
    )?;
    invalidate_project_workspace_context(project_id);
    Ok(())
}
''',
)
replace_once(
    "src-tauri/src/local_workspace.rs",
    '''    fs::write(&file_path, content.as_bytes())?;
    Ok(WorkspaceMutationResult {
''',
    '''    fs::write(&file_path, content.as_bytes())?;
    invalidate_project_workspace_context(&project_id);
    Ok(WorkspaceMutationResult {
''',
)
replace_once(
    "src-tauri/src/local_workspace.rs",
    '''    fs::create_dir_all(&directory)?;
    Ok(WorkspaceMutationResult {
''',
    '''    fs::create_dir_all(&directory)?;
    invalidate_project_workspace_context(&project_id);
    Ok(WorkspaceMutationResult {
''',
)
replace_once(
    "src-tauri/src/local_workspace.rs",
    '''    let kind = if source.is_dir() { "directory" } else { "file" };
    fs::rename(&source, &target)?;
    Ok(WorkspaceMutationResult {
''',
    '''    let kind = if source.is_dir() { "directory" } else { "file" };
    fs::rename(&source, &target)?;
    invalidate_project_workspace_context(&project_id);
    Ok(WorkspaceMutationResult {
''',
)
replace_once(
    "src-tauri/src/local_workspace.rs",
    '''    if target.is_dir() {
        fs::remove_dir_all(target)?;
    } else {
        fs::remove_file(target)?;
    }
    Ok(())
}
''',
    '''    if target.is_dir() {
        fs::remove_dir_all(target)?;
    } else {
        fs::remove_file(target)?;
    }
    invalidate_project_workspace_context(&project_id);
    Ok(())
}
''',
)

# P0: do not infer expensive Thinking mode from ordinary debug/solve wording.
replace_once(
    "src/lib/chat.ts",
    '''  if (/\\b(think|reason|solve|debug|step by step|carefully)\\b/.test(text)) return "thinking";
''',
    '''  if (/\\b(think deeply|reason deeply|reason carefully|step by step|show your reasoning)\\b/.test(text))
    return "thinking";
''',
)

# P3: Tauri stream events are authoritative. Remove the 1.5 s full-message
# polling loop and avoid the expensive full app/runtime inventory refresh after
# every completed response.
replace_once(
    "src/App.tsx",
    '''  useEffect(() => {
    if (!activeId || !streamingId) return;
    const interval = window.setInterval(() => {
      api
        .messages(activeId)
        .then((items) => {
          const visible = items.filter((message) => message.role !== "system");
          setMessages((current) => mergeStreamingSnapshot(current, visible));
          const streamedMessage = visible.find((message) => message.id === streamingId);
          if (streamedMessage && streamedMessage.status !== "streaming") {
            setStreamingId(null);
          }
        })
        .catch(showError);
    }, 1500);
    return () => window.clearInterval(interval);
  }, [activeId, showError, streamingId]);

''',
    '',
)
replace_all_exact(
    "src/App.tsx",
    '''      await refreshMessages(conversationId, setMessages, showError);
      await refreshApp();
''',
    '''      await refreshMessages(conversationId, setMessages, showError);
      setConversations(await api.conversations());
''',
    2,
)

# mergeStreamingSnapshot is no longer used after removing polling.
replace_once(
    "src/App.tsx",
    '''  mergeStreamingSnapshot,
''',
    '',
)

print("Applied P0-P3 token-generation latency fixes.")
