use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use reqwest::Client;
use serde_json::json;
use sysinfo::System;
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;

use crate::{
    app_error::AppError,
    chat::ChatRepository,
    launch_planner::ModelLaunchPlanner,
    model_registry::{ModelRecord, ModelRegistry},
    runtime::allocate_local_port,
    AppState,
};

const CORE_MODEL_REPOSITORY: &str = "Qwen/Qwen3-4B-GGUF";
const STARTUP_WARM_DELAY_MS: u64 = 150;
const MEMORY_CHECK_INTERVAL_SECS: u64 = 60;
const IDLE_BEFORE_MEMORY_TRIM_SECS: u64 = 20 * 60;
const LOW_MEMORY_MIN_AVAILABLE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const LOW_MEMORY_AVAILABLE_PERCENT: u64 = 10;
const WARMUP_REQUEST_TIMEOUT_SECS: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WarmPhase {
    Idle,
    Loading,
    Ready,
    Failed,
}

#[derive(Debug)]
struct WarmState {
    phase: WarmPhase,
    model_id: Option<String>,
    foreground_model_id: Option<String>,
    last_foreground_use: Option<Instant>,
}

impl Default for WarmState {
    fn default() -> Self {
        Self {
            phase: WarmPhase::Idle,
            model_id: None,
            foreground_model_id: None,
            last_foreground_use: None,
        }
    }
}

#[derive(Default)]
pub struct WarmStartCoordinator {
    state: Mutex<WarmState>,
    notify: Notify,
}

impl WarmStartCoordinator {
    pub fn note_foreground_request(&self, model_id: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.foreground_model_id = Some(model_id.to_string());
            state.last_foreground_use = Some(Instant::now());
        }
    }

    pub async fn wait_if_loading(&self, model_id: &str) {
        loop {
            let notified = self.notify.notified();
            let should_wait = self.state.lock().is_ok_and(|state| {
                state.phase == WarmPhase::Loading && state.model_id.as_deref() == Some(model_id)
            });
            if !should_wait {
                return;
            }
            notified.await;
        }
    }

    pub fn mark_runtime_ready(&self, model_id: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.phase = WarmPhase::Ready;
            state.model_id = Some(model_id.to_string());
        }
        self.notify.notify_waiters();
    }

    pub fn mark_runtime_stopped(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.phase = WarmPhase::Idle;
            state.model_id = None;
        }
        self.notify.notify_waiters();
    }

    fn begin_background_load(&self, model_id: &str) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.foreground_model_id.is_some() || state.phase == WarmPhase::Loading {
            return false;
        }
        state.phase = WarmPhase::Loading;
        state.model_id = Some(model_id.to_string());
        true
    }

    fn finish_background_load(&self, model_id: &str, success: bool) {
        if let Ok(mut state) = self.state.lock() {
            if state.model_id.as_deref() == Some(model_id) {
                state.phase = if success {
                    state.last_foreground_use.get_or_insert_with(Instant::now);
                    WarmPhase::Ready
                } else {
                    WarmPhase::Failed
                };
            }
        }
        self.notify.notify_waiters();
    }

    fn foreground_requested(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.foreground_model_id.is_some())
    }

    fn idle_model_for_memory_trim(&self, minimum_idle: Duration) -> Option<String> {
        let state = self.state.lock().ok()?;
        if state.phase != WarmPhase::Ready {
            return None;
        }
        let last_use = state.last_foreground_use?;
        (last_use.elapsed() >= minimum_idle)
            .then(|| state.model_id.clone())
            .flatten()
    }

    fn mark_unloaded(&self, model_id: &str) {
        if let Ok(mut state) = self.state.lock() {
            if state.model_id.as_deref() == Some(model_id) {
                state.phase = WarmPhase::Idle;
                state.model_id = None;
            }
        }
    }
}

pub fn spawn_background_services(app: AppHandle) {
    spawn_startup_warmup(app.clone());
    spawn_memory_pressure_monitor(app);
}

fn spawn_startup_warmup(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(STARTUP_WARM_DELAY_MS)).await;

        let model = {
            let state = app.state::<AppState>();
            if state.warm_start.foreground_requested() {
                tracing::debug!(
                    "startup model warmup skipped because foreground chat already started"
                );
                return;
            }
            match select_startup_model(&state) {
                Ok(Some(model)) => model,
                Ok(None) => {
                    tracing::debug!(
                        "startup model warmup skipped because no usable local chat model is installed"
                    );
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "could not select startup warm model");
                    return;
                }
            }
        };

        if !is_core_model(&model) {
            tracing::info!(
                model = %model.name,
                "OpenMindAI Core unavailable; using installed local chat model for warm start"
            );
        }

        {
            let state = app.state::<AppState>();
            if !state.warm_start.begin_background_load(&model.id) {
                return;
            }
        }

        let model_id = model.id.clone();
        let model_name = model.name.clone();
        let load_app = app.clone();
        let load_model = model.clone();
        let loaded = tauri::async_runtime::spawn_blocking(move || {
            let state = load_app.state::<AppState>();
            if state.warm_start.foreground_requested() {
                return Ok(None);
            }
            let hardware = state.hardware.clone();
            let plan = ModelLaunchPlanner::plan(&load_model, &hardware, allocate_local_port()?);
            let mut runtime = state
                .runtime
                .lock()
                .map_err(|_| AppError::internal("runtime lock poisoned"))?;
            let status = runtime.ensure_model_server(&hardware, &plan.config)?;
            Ok::<_, AppError>(status.endpoint)
        })
        .await;

        let endpoint = match loaded {
            Ok(Ok(Some(endpoint))) => {
                app.state::<AppState>()
                    .warm_start
                    .finish_background_load(&model_id, true);
                tracing::info!(model = %model_name, "startup model preloaded and kept warm");
                endpoint
            }
            Ok(Ok(None)) => {
                app.state::<AppState>()
                    .warm_start
                    .finish_background_load(&model_id, false);
                return;
            }
            Ok(Err(error)) => {
                app.state::<AppState>()
                    .warm_start
                    .finish_background_load(&model_id, false);
                tracing::warn!(%error, model = %model_name, "startup model warmup failed");
                return;
            }
            Err(error) => {
                app.state::<AppState>()
                    .warm_start
                    .finish_background_load(&model_id, false);
                tracing::warn!(%error, model = %model_name, "startup warmup worker failed");
                return;
            }
        };

        // Loading the weights removes the dominant cold-start cost. If no user
        // has started a chat yet, run one tiny local request to prime kernels and
        // the llama-server prompt-cache path. Never make a user wait behind it.
        let (client, foreground_started) = {
            let state = app.state::<AppState>();
            (state.http.clone(), state.warm_start.foreground_requested())
        };
        if !foreground_started {
            prime_local_server(&client, &endpoint, &model).await;
        }
    });
}

fn select_startup_model(state: &AppState) -> Result<Option<ModelRecord>, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let models = ModelRegistry::new(&db, &state.root).list_models()?;
    if models.is_empty() {
        return Ok(None);
    }

    let conversations = ChatRepository::new(&db).list_conversations()?;
    let last_active_model_id = conversations
        .iter()
        .max_by(|left, right| left.updated_at.cmp(&right.updated_at))
        .and_then(|conversation| conversation.active_model_id.as_deref());

    Ok(choose_startup_model(&models, last_active_model_id))
}

fn choose_startup_model(
    models: &[ModelRecord],
    last_active_model_id: Option<&str>,
) -> Option<ModelRecord> {
    // OpenMindAI Core stays the first choice. If it is missing, choose another
    // usable installed local chat model rather than leaving startup without a
    // warm runtime. Prefer text-only models before multimodal models and choose
    // the smallest candidate inside a fallback tier to limit RAM/VRAM pressure.
    let last_core = last_active_model_id.and_then(|id| {
        models
            .iter()
            .find(|model| model.id == id && is_core_model(model))
            .cloned()
    });
    if last_core.is_some() {
        return last_core;
    }

    if let Some(core) = models.iter().find(|model| is_core_model(model)).cloned() {
        return Some(core);
    }

    let text_chat = models
        .iter()
        .filter(|model| is_usable_local_chat_model(model) && !is_multimodal_model(model));
    if let Some(model) = smallest_model(text_chat) {
        return Some(model);
    }

    smallest_model(
        models
            .iter()
            .filter(|model| is_usable_local_chat_model(model)),
    )
}

fn smallest_model<'a>(models: impl Iterator<Item = &'a ModelRecord>) -> Option<ModelRecord> {
    models.min_by_key(|model| model.size_bytes.max(0)).cloned()
}

fn is_core_model(model: &ModelRecord) -> bool {
    is_usable_local_chat_model(model)
        && model.source_repository.as_deref() == Some(CORE_MODEL_REPOSITORY)
}

fn is_usable_local_chat_model(model: &ModelRecord) -> bool {
    model.enabled
        && model.format.eq_ignore_ascii_case("gguf")
        && model.verification.as_deref() != Some("failed")
        && has_capability(model, "chat")
}

fn is_multimodal_model(model: &ModelRecord) -> bool {
    ["vision", "image", "video", "voice", "audio", "ocr"]
        .iter()
        .any(|capability| has_capability(model, capability))
}

fn has_capability(model: &ModelRecord, capability: &str) -> bool {
    serde_json::from_str::<Vec<String>>(&model.capabilities)
        .map(|capabilities| {
            capabilities
                .iter()
                .any(|current| current.eq_ignore_ascii_case(capability))
        })
        .unwrap_or(false)
}

async fn prime_local_server(client: &Client, endpoint: &str, model: &ModelRecord) {
    let body = json!({
        "model": model.id.as_str(),
        "messages": [{"role": "user", "content": "Hi"}],
        "stream": false,
        "temperature": 0.0,
        "max_tokens": 1,
        "cache_prompt": true
    });
    let result = client
        .post(format!("{endpoint}/v1/chat/completions"))
        .json(&body)
        .timeout(Duration::from_secs(WARMUP_REQUEST_TIMEOUT_SECS))
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => {
            tracing::debug!("local model warmup inference completed")
        }
        Ok(response) => {
            tracing::debug!(status = %response.status(), "local warmup inference was not accepted")
        }
        Err(error) => tracing::debug!(%error, "local warmup inference skipped after preload"),
    }
}

fn spawn_memory_pressure_monitor(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let idle_limit = Duration::from_secs(IDLE_BEFORE_MEMORY_TRIM_SECS);
        loop {
            tokio::time::sleep(Duration::from_secs(MEMORY_CHECK_INTERVAL_SECS)).await;

            let model_id = {
                let state = app.state::<AppState>();
                let Some(model_id) = state.warm_start.idle_model_for_memory_trim(idle_limit) else {
                    continue;
                };
                if !state.active_generations.is_idle() {
                    continue;
                }
                model_id
            };

            let mut system = System::new();
            system.refresh_memory();
            let available = system.available_memory();
            let total = system.total_memory();
            let low_percent =
                total > 0 && available.saturating_mul(100) / total <= LOW_MEMORY_AVAILABLE_PERCENT;
            if available > LOW_MEMORY_MIN_AVAILABLE_BYTES && !low_percent {
                continue;
            }

            let stop_app = app.clone();
            let stopped = tauri::async_runtime::spawn_blocking(move || {
                let state = stop_app.state::<AppState>();
                if !state.active_generations.is_idle() {
                    return Ok(false);
                }
                let mut runtime = state
                    .runtime
                    .lock()
                    .map_err(|_| AppError::internal("runtime lock poisoned"))?;
                runtime.stop()?;
                Ok::<_, AppError>(true)
            })
            .await;

            if matches!(stopped, Ok(Ok(true))) {
                app.state::<AppState>().warm_start.mark_unloaded(&model_id);
                tracing::info!(
                    available_memory_bytes = available,
                    model_id = %model_id,
                    "idle local model unloaded because system memory is low"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn waits_only_for_same_loading_model() {
        let coordinator = WarmStartCoordinator::default();
        assert!(coordinator.begin_background_load("core"));
        coordinator.wait_if_loading("other").await;
        coordinator.finish_background_load("core", true);
        coordinator.wait_if_loading("core").await;
    }

    #[test]
    fn foreground_request_prevents_background_start() {
        let coordinator = WarmStartCoordinator::default();
        coordinator.note_foreground_request("core");
        assert!(!coordinator.begin_background_load("core"));
    }

    fn test_model(
        id: &str,
        repo: Option<&str>,
        size_bytes: i64,
        capabilities: &[&str],
    ) -> ModelRecord {
        ModelRecord {
            id: id.to_string(),
            name: id.to_string(),
            family: None,
            path: format!("models/{id}.gguf"),
            format: "gguf".to_string(),
            quantization: None,
            size_bytes,
            capabilities: serde_json::to_string(capabilities).unwrap(),
            context_length: None,
            preferred_backend: Some("vulkan".to_string()),
            enabled: true,
            source_repository: repo.map(ToString::to_string),
            verification: Some("verified".to_string()),
            state: crate::model_registry::ModelLifecycleState::Ready,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
        }
    }

    #[test]
    fn core_model_wins_when_available() {
        let fallback = test_model("small", None, 1_000, &["chat"]);
        let core = test_model("core", Some(CORE_MODEL_REPOSITORY), 2_000, &["chat"]);
        let selected = choose_startup_model(&[fallback, core], None).unwrap();
        assert_eq!(selected.id, "core");
    }

    #[test]
    fn missing_core_falls_back_to_smallest_text_chat_model() {
        let large = test_model("large", None, 8_000, &["chat", "thinking"]);
        let small = test_model("small", None, 3_000, &["chat"]);
        let vision = test_model("vision", None, 2_000, &["chat", "vision"]);
        let selected = choose_startup_model(&[large, small, vision], None).unwrap();
        assert_eq!(selected.id, "small");
    }

    #[test]
    fn multimodal_chat_is_used_when_it_is_the_only_available_chat_model() {
        let vision = test_model("vision", None, 4_000, &["chat", "vision"]);
        let selected = choose_startup_model(&[vision], None).unwrap();
        assert_eq!(selected.id, "vision");
    }

    #[test]
    fn disabled_failed_and_non_chat_models_are_not_warm_candidates() {
        let mut disabled = test_model("disabled", None, 1_000, &["chat"]);
        disabled.enabled = false;
        let mut failed = test_model("failed", None, 2_000, &["chat"]);
        failed.verification = Some("failed".to_string());
        let vision_only = test_model("vision-only", None, 500, &["vision"]);
        assert!(choose_startup_model(&[disabled, failed, vision_only], None).is_none());
    }
}
