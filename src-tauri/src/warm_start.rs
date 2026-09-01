use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

#[cfg(target_os = "windows")]
use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
};

use sysinfo::System;
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;

use crate::{
    app_error::AppError,
    hardware::HardwareProfiler,
    launch_planner::ModelLaunchPlanner,
    model_registry::{ModelLifecycleState, ModelRegistry},
    runtime::{allocate_local_port, LlamaRuntimeStatus},
    AppState,
};

const MEMORY_CHECK_INTERVAL_SECS: u64 = 60;
const IDLE_BEFORE_MEMORY_TRIM_SECS: u64 = 20 * 60;
const LOW_MEMORY_MIN_AVAILABLE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const LOW_MEMORY_AVAILABLE_PERCENT: u64 = 10;
const CORE_REPOSITORY: &str = "Qwen/Qwen3-4B-GGUF";
const BACKGROUND_ENV: &str = "OPENMINDAI_BACKGROUND_BOOT";
const BACKGROUND_PRELOAD_DELAY_SECS: u64 = 8;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

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

fn is_background_boot() -> bool {
    std::env::var_os(BACKGROUND_ENV).is_some()
}

fn prepare_default_chat_runtime_sync(app: &AppHandle) -> Result<LlamaRuntimeStatus, AppError> {
    let state = app.state::<AppState>();
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
                    && matches!(
                        model.state,
                        ModelLifecycleState::Ready | ModelLifecycleState::Loaded
                    )
                    && model.source_repository.as_deref() == Some(CORE_REPOSITORY)
            })
            .ok_or_else(|| {
                AppError::ModelNotFound(
                    "OpenMindAI Core is not installed or ready for local chat".to_string(),
                )
            })?
    };

    // Keep runtime selection and launch planning on the same completed hardware
    // profile. This matters when the loader races the background DXGI scan:
    // NVIDIA must not accidentally select Vulkan and AMD/Windows must not fall
    // back to CPU just because the initial startup snapshot had no GPU yet.
    let hardware = HardwareProfiler::for_inference(&state.hardware);
    let plan = ModelLaunchPlanner::plan(&model, &hardware, allocate_local_port()?);
    let status = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| AppError::internal("runtime lock poisoned"))?;
        runtime.ensure_model_server(&hardware, &plan.config)?
    };
    state.warm_start.mark_runtime_ready(&model.id);
    Ok(status)
}

/// Used by the startup overlay on a normal launch and by the hidden Windows
/// login instance. A background-login preload waits briefly so Windows can
/// finish its own login work before OpenMindAI allocates model memory/VRAM.
#[tauri::command]
pub(crate) async fn prepare_default_chat_runtime(
    app: AppHandle,
) -> Result<LlamaRuntimeStatus, AppError> {
    if is_background_boot() {
        tokio::time::sleep(Duration::from_secs(BACKGROUND_PRELOAD_DELAY_SECS)).await;
    }

    tauri::async_runtime::spawn_blocking(move || prepare_default_chat_runtime_sync(&app))
        .await
        .map_err(|error| AppError::internal(format!("startup model task failed: {error}")))?
}

/// The native window starts hidden. React calls this only after the staged
/// loader has mounted, which prevents a raw WebView/white frame from ever being
/// shown. Windows-login background instances intentionally remain hidden.
#[tauri::command]
pub(crate) fn reveal_main_window(app: AppHandle) -> Result<bool, AppError> {
    if is_background_boot() {
        return Ok(false);
    }
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| AppError::internal("main window is unavailable"))?;
    window
        .show()
        .map_err(|error| AppError::internal(error.to_string()))?;
    window
        .set_focus()
        .map_err(|error| AppError::internal(error.to_string()))?;
    Ok(true)
}

/// The process can be launched with `--background` from Windows login. The
/// same process stays alive with its Core model/runtime resident; a later
/// normal launch is handed to this instance by `main.rs` and simply reveals
/// the already-prepared Tauri window.
pub fn spawn_background_services(app: AppHandle) {
    if is_background_boot() {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.hide();
        }
    }

    register_windows_autostart();
    spawn_memory_pressure_monitor(app);
}

#[cfg(target_os = "windows")]
fn register_windows_autostart() {
    if cfg!(debug_assertions) {
        return;
    }
    let Some(config) = crate::portable_root::load_installation() else {
        return;
    };
    if !config.installation_complete {
        return;
    }

    std::thread::spawn(|| {
        let Ok(executable) = std::env::current_exe() else {
            return;
        };
        let command_value = format!("\"{}\" --background", executable.display());
        let mut command = Command::new("reg.exe");
        command
            .args([
                "add",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                "/v",
                "OpenMindAI",
                "/t",
                "REG_SZ",
                "/d",
                &command_value,
                "/f",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        match command.status() {
            Ok(status) if status.success() => {
                tracing::debug!("Windows login background startup registered");
            }
            Ok(status) => {
                tracing::warn!(?status, "could not register Windows login startup");
            }
            Err(error) => {
                tracing::warn!(%error, "could not launch reg.exe for Windows startup registration");
            }
        }
    });
}

#[cfg(not(target_os = "windows"))]
fn register_windows_autostart() {}

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
    async fn foreground_request_never_waits_for_startup_prepare() {
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
