use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use sysinfo::System;
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;

use crate::{app_error::AppError, AppState};

const MEMORY_CHECK_INTERVAL_SECS: u64 = 60;
const IDLE_BEFORE_MEMORY_TRIM_SECS: u64 = 20 * 60;
const LOW_MEMORY_MIN_AVAILABLE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const LOW_MEMORY_AVAILABLE_PERCENT: u64 = 10;

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

/// Startup/background work must never load GGUF weights speculatively. Model
/// loading is user-triggered by the first prompt or explicit model activation,
/// so opening OpenMindAI cannot suddenly saturate GPU/VRAM and stall the
/// desktop. The only always-on background service is the low-memory idle
/// runtime monitor.
pub fn spawn_background_services(app: AppHandle) {
    spawn_memory_pressure_monitor(app);
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
    async fn foreground_request_never_waits_for_speculative_preload() {
        let coordinator = WarmStartCoordinator::default();
        coordinator.note_foreground_request("core");
        coordinator.wait_if_loading("core").await;
        coordinator.mark_runtime_ready("core");
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
