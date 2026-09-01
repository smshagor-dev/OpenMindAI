use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use sysinfo::System;
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;

use crate::{
    app_error::AppError,
    hardware::HardwareProfiler,
    launch_planner::ModelLaunchPlanner,
    model_registry::ModelRegistry,
    runtime::allocate_local_port,
    AppState,
};

const CORE_IDLE_WARMUP_DELAY_MS: u64 = 1_500;
const MEMORY_CHECK_INTERVAL_SECS: u64 = 60;
const IDLE_BEFORE_MEMORY_TRIM_SECS: u64 = 20 * 60;
const LOW_MEMORY_MIN_AVAILABLE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const LOW_MEMORY_AVAILABLE_PERCENT: u64 = 10;
const CORE_REPOSITORY: &str = "Qwen/Qwen3-4B-GGUF";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WarmPhase {
    Idle,
    Loading,
    Ready,
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
            if state.model_id.as_deref() != Some(model_id) {
                state.phase = WarmPhase::Loading;
                state.model_id = Some(model_id.to_string());
            }
        }
        self.notify.notify_waiters();
    }

    pub async fn wait_if_loading(&self, model_id: &str) {
        loop {
            let notified = self.notify.notified();
            let should_wait = self.state.lock().is_ok_and(|state| {
                state.phase == WarmPhase::Loading
                    && state.model_id.as_deref() == Some(model_id)
                    && state.foreground_model_id.as_deref() != Some(model_id)
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
            state.last_foreground_use.get_or_insert_with(Instant::now);
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

    fn has_foreground_request(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.foreground_model_id.is_some())
    }

    fn begin_background_warmup(&self, model_id: &str) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.foreground_model_id.is_some() || state.phase != WarmPhase::Idle {
            return false;
        }
        state.phase = WarmPhase::Loading;
        state.model_id = Some(model_id.to_string());
        true
    }

    fn clear_background_warmup(&self, model_id: &str) {
        if let Ok(mut state) = self.state.lock() {
            if state.phase == WarmPhase::Loading
                && state.model_id.as_deref() == Some(model_id)
                && state.foreground_model_id.is_none()
            {
                state.phase = WarmPhase::Idle;
                state.model_id = None;
            }
        }
        self.notify.notify_waiters();
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

/// Keep first paint fast, then warm only the small default Core model in the
/// background. This moves GGUF loading off the window-creation path while still
/// making the common first chat turn hot by the time a user finishes typing.
pub fn spawn_background_services(app: AppHandle) {
    spawn_core_idle_warmup(app.clone());
    spawn_memory_pressure_monitor(app);
}

fn spawn_core_idle_warmup(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(CORE_IDLE_WARMUP_DELAY_MS)).await;

        let warm_app = app.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let state = warm_app.state::<AppState>();
            if !state.active_generations.is_idle() || state.warm_start.has_foreground_request() {
                return Ok::<_, AppError>(None);
            }

            let model = {
                let db = state
                    .database
                    .lock()
                    .map_err(|_| AppError::internal("database lock poisoned"))?;
                ModelRegistry::new(&db, &state.root)
                    .list_models()?
                    .into_iter()
                    .find(|model| {
                        model.enabled
                            && model.source_repository.as_deref() == Some(CORE_REPOSITORY)
                    })
            };
            let Some(model) = model else {
                return Ok(None);
            };

            if !state.warm_start.begin_background_warmup(&model.id) {
                return Ok(None);
            }

            // A foreground prompt always wins. Check between each potentially
            // expensive preparation step so a user who sends immediately does
            // not queue behind unnecessary speculative work.
            if state.warm_start.has_foreground_request() {
                state.warm_start.clear_background_warmup(&model.id);
                return Ok(None);
            }
            let hardware = HardwareProfiler::for_inference(&state.hardware);
            if state.warm_start.has_foreground_request() {
                state.warm_start.clear_background_warmup(&model.id);
                return Ok(None);
            }
            let plan = ModelLaunchPlanner::plan(&model, &hardware, allocate_local_port()?);
            if state.warm_start.has_foreground_request() {
                state.warm_start.clear_background_warmup(&model.id);
                return Ok(None);
            }

            let status = {
                let mut runtime = state
                    .runtime
                    .lock()
                    .map_err(|_| AppError::internal("runtime lock poisoned"))?;
                runtime.ensure_model_server(&hardware, &plan.config)
            };

            match status {
                Ok(status) => {
                    state.warm_start.mark_runtime_ready(&model.id);
                    Ok(Some((model.name, status.backend)))
                }
                Err(error) => {
                    state.warm_start.clear_background_warmup(&model.id);
                    Err(error)
                }
            }
        })
        .await;

        match result {
            Ok(Ok(Some((model_name, backend)))) => {
                tracing::info!(
                    model = %model_name,
                    ?backend,
                    "default chat model warmed after first paint"
                );
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, "background chat model warmup skipped after failure");
            }
            Err(error) => {
                tracing::warn!(%error, "background chat model warmup task failed");
            }
        }
    });
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
    async fn foreground_request_never_waits_for_background_preload() {
        let coordinator = WarmStartCoordinator::default();
        coordinator.note_foreground_request("core");
        coordinator.wait_if_loading("core").await;
        coordinator.mark_runtime_ready("core");
    }

    #[test]
    fn foreground_request_prevents_speculative_warmup() {
        let coordinator = WarmStartCoordinator::default();
        coordinator.note_foreground_request("core");
        assert!(!coordinator.begin_background_warmup("core"));
    }

    #[test]
    fn ready_runtime_is_tracked_for_idle_trim() {
        let coordinator = WarmStartCoordinator::default();
        coordinator.note_foreground_request("core");
        coordinator.mark_runtime_ready("core");
        assert!(coordinator
            .idle_model_for_memory_trim(Duration::from_secs(0))
            .is_some());
    }
}
