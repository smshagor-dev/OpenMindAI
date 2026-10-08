//! Runtime lifecycle for the OpenAgent coding model (Settings -> Agent Setup).
//!
//! Every caller that needs the coding agent (the VS Code coding-agent endpoint, desktop
//! OpenAgent runs and the Agent Setup status view) goes through [`acquire_agent_runtime`].
//! It decides where the agent runs (sharing the Core llama-server or in its own dedicated
//! server, see [`decide_placement`]), starts the model without holding the runtime lock while
//! it loads, and lets concurrent callers wait on one startup instead of launching duplicates.

use std::{
    future::Future,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};
use tokio::time::Instant;

use crate::{
    app_error::AppError,
    hardware::{BackendKind, HardwareProfiler},
    launch_planner::{estimate_context_bytes, ModelLaunchConfig, ModelLaunchPlanner},
    local_agent,
    model_registry::{ModelRecord, ModelRegistry},
    runtime::{
        allocate_local_port, LlamaRuntimeManager, ModelLaunch, ModelLease, ModelProbe, ServerState,
    },
    settings::SettingsRepository,
    warm_start, AppState,
};

/// Upper bound for one agent model load. Cold loads of a 4B GGUF from disk were observed
/// at about 2.5 minutes on a Vulkan RX 580, beyond the 120 s chat-path limit.
pub const AGENT_STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Runtime overhead the launch planner adds on top of weights and KV cache.
const RUNTIME_OVERHEAD_BYTES: u64 = 768 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Startup coordination
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentPhase {
    Disabled,
    NotInstalled,
    Stopped,
    Starting,
    LoadingModel,
    Ready,
    Error,
}

pub enum Launch {
    /// The model is resident and healthy at this endpoint.
    Ready(String),
    /// A load is in progress; poll [`AgentLauncher::probe`].
    Started,
    /// The runtime is still loading a different model (shared runtime); retry later.
    Busy,
}

/// Starts and checks one model server. Implemented by the real runtime and by test fakes.
pub trait AgentLauncher {
    fn launch(&self) -> impl Future<Output = Result<Launch, AppError>> + Send;
    fn probe(&self) -> impl Future<Output = ModelProbe> + Send;
    fn abort(&self) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Clone)]
struct StartupRecord {
    model_key: String,
    phase: AgentPhase,
    started_at: Instant,
    last_startup: Option<Duration>,
    error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct StartupView {
    pub model_key: Option<String>,
    pub phase: Option<AgentPhase>,
    pub elapsed: Option<Duration>,
    pub last_startup: Option<Duration>,
    pub error: Option<String>,
}

/// Serializes agent startups. Callers that arrive while a load is running wait for it and
/// then reuse the resident model; if that load failed they get the same error instead of
/// starting another full load.
#[derive(Default)]
pub struct AgentStartup {
    gate: tokio::sync::Mutex<()>,
    record: std::sync::Mutex<Option<StartupRecord>>,
    completed: AtomicU64,
    last_use: std::sync::Mutex<Option<Instant>>,
}

impl AgentStartup {
    /// Shows a pending start immediately, before a background task reaches the gate.
    pub fn mark_requested(&self, model_key: &str) {
        let Ok(mut record) = self.record.lock() else {
            return;
        };
        let in_progress = record.as_ref().is_some_and(|current| {
            current.model_key == model_key
                && matches!(
                    current.phase,
                    AgentPhase::Starting | AgentPhase::LoadingModel
                )
        });
        if !in_progress {
            let last_startup = record.as_ref().and_then(|current| current.last_startup);
            *record = Some(StartupRecord {
                model_key: model_key.to_string(),
                phase: AgentPhase::Starting,
                started_at: Instant::now(),
                last_startup,
                error: None,
            });
        }
    }

    pub fn view(&self) -> StartupView {
        let Some(record) = self.record.lock().ok().and_then(|record| record.clone()) else {
            return StartupView::default();
        };
        let loading = matches!(
            record.phase,
            AgentPhase::Starting | AgentPhase::LoadingModel
        );
        StartupView {
            model_key: Some(record.model_key),
            phase: Some(record.phase),
            elapsed: loading.then(|| record.started_at.elapsed()),
            last_startup: record.last_startup,
            error: record.error,
        }
    }

    /// Time since the agent was last acquired, used for idle memory trimming.
    pub fn idle_for(&self) -> Option<Duration> {
        self.last_use
            .lock()
            .ok()
            .and_then(|last| last.map(|at| at.elapsed()))
    }

    pub async fn acquire<L: AgentLauncher>(
        &self,
        model_key: &str,
        launcher: &L,
        timeout: Duration,
    ) -> Result<String, AppError> {
        let seen = self.completed.load(Ordering::SeqCst);
        let _gate = self.gate.lock().await;
        if let Ok(mut last) = self.last_use.lock() {
            *last = Some(Instant::now());
        }
        if self.completed.load(Ordering::SeqCst) != seen {
            // Another caller ran a startup while this one waited; share a failed outcome.
            if let Some(error) = self.failed_error(model_key) {
                return Err(AppError::ModelLoadFailed(error));
            }
        }
        let result = self.start_locked(model_key, launcher, timeout).await;
        self.completed.fetch_add(1, Ordering::SeqCst);
        result
    }

    async fn start_locked<L: AgentLauncher>(
        &self,
        model_key: &str,
        launcher: &L,
        timeout: Duration,
    ) -> Result<String, AppError> {
        let started = self.begin(model_key);
        loop {
            match launcher.launch().await {
                Err(error) => {
                    self.finish(
                        model_key,
                        AgentPhase::Error,
                        started,
                        Some(error.to_string()),
                    );
                    return Err(error);
                }
                Ok(Launch::Ready(endpoint)) => {
                    self.finish_resident(model_key);
                    return Ok(endpoint);
                }
                Ok(Launch::Started) => {
                    self.set_phase(model_key, AgentPhase::LoadingModel);
                    break;
                }
                // The shared runtime is still loading another model (for example Core).
                // Wait for that load instead of killing it, then switch.
                Ok(Launch::Busy) => {
                    if started.elapsed() >= timeout {
                        let reason =
                            "the shared runtime stayed busy loading another model".to_string();
                        self.finish(model_key, AgentPhase::Error, started, Some(reason.clone()));
                        return Err(AppError::ModelLoadFailed(reason));
                    }
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            }
        }
        loop {
            match launcher.probe().await {
                ModelProbe::Ready(endpoint) => {
                    self.finish(model_key, AgentPhase::Ready, started, None);
                    return Ok(endpoint);
                }
                ModelProbe::Failed(reason) => {
                    self.finish(model_key, AgentPhase::Error, started, Some(reason.clone()));
                    return Err(AppError::ModelLoadFailed(reason));
                }
                ModelProbe::Loading => {}
            }
            if started.elapsed() >= timeout {
                launcher.abort().await;
                let reason = format!(
                    "the coding agent model did not finish loading within {} seconds",
                    timeout.as_secs()
                );
                self.finish(model_key, AgentPhase::Error, started, Some(reason.clone()));
                return Err(AppError::ModelLoadFailed(reason));
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    /// Records the start of an attempt, keeping an earlier `mark_requested` time so the
    /// elapsed time shown to users covers the whole wait.
    fn begin(&self, model_key: &str) -> Instant {
        let Ok(mut record) = self.record.lock() else {
            return Instant::now();
        };
        if let Some(current) = record.as_ref() {
            if current.model_key == model_key && current.phase == AgentPhase::Starting {
                return current.started_at;
            }
        }
        let last_startup = record.as_ref().and_then(|current| current.last_startup);
        let now = Instant::now();
        *record = Some(StartupRecord {
            model_key: model_key.to_string(),
            phase: AgentPhase::Starting,
            started_at: now,
            last_startup,
            error: None,
        });
        now
    }

    fn set_phase(&self, model_key: &str, phase: AgentPhase) {
        if let Ok(mut record) = self.record.lock() {
            if let Some(current) = record
                .as_mut()
                .filter(|current| current.model_key == model_key)
            {
                current.phase = phase;
            }
        }
    }

    fn finish(&self, model_key: &str, phase: AgentPhase, started: Instant, error: Option<String>) {
        if let Ok(mut record) = self.record.lock() {
            *record = Some(StartupRecord {
                model_key: model_key.to_string(),
                phase,
                started_at: started,
                last_startup: (phase == AgentPhase::Ready).then(|| started.elapsed()),
                error,
            });
        }
    }

    fn finish_resident(&self, model_key: &str) {
        if let Ok(mut record) = self.record.lock() {
            let last_startup = record
                .as_ref()
                .filter(|current| current.model_key == model_key)
                .and_then(|current| current.last_startup);
            *record = Some(StartupRecord {
                model_key: model_key.to_string(),
                phase: AgentPhase::Ready,
                started_at: Instant::now(),
                last_startup,
                error: None,
            });
        }
    }

    fn failed_error(&self, model_key: &str) -> Option<String> {
        let record = self.record.lock().ok()?;
        let current = record.as_ref()?;
        (current.model_key == model_key && current.phase == AgentPhase::Error)
            .then(|| current.error.clone())
            .flatten()
    }
}

// ---------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeMode {
    Automatic,
    Shared,
    Dedicated,
}

impl RuntimeMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "shared" => Self::Shared,
            "dedicated" => Self::Dedicated,
            _ => Self::Automatic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Placement {
    /// The agent uses the same llama-server as OpenMindAI Core (models swap).
    Shared,
    /// The agent has its own llama-server next to Core.
    Dedicated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuUse {
    None,
    Full,
    /// Partial offload means the planner sized the model to fill the GPU budget by itself.
    Partial,
}

#[derive(Debug, Clone, Copy)]
pub struct Footprint {
    pub bytes: u64,
    pub gpu: GpuUse,
}

impl Footprint {
    pub fn from_config(model_bytes: u64, config: &ModelLaunchConfig) -> Self {
        let gpu = if config.gpu_layers <= 0 || config.backend == BackendKind::Cpu {
            GpuUse::None
        } else if config.gpu_layers >= 999 {
            GpuUse::Full
        } else {
            GpuUse::Partial
        };
        Self {
            bytes: model_bytes
                .saturating_add(estimate_context_bytes(config.context_size))
                .saturating_add(RUNTIME_OVERHEAD_BYTES),
            gpu,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacementDecision {
    pub placement: Placement,
    /// The agent runs without GPU offload so Core keeps the GPU memory.
    pub agent_on_cpu: bool,
    pub reason: String,
}

impl PlacementDecision {
    fn new(placement: Placement, agent_on_cpu: bool, reason: &str) -> Self {
        Self {
            placement,
            agent_on_cpu,
            reason: reason.to_string(),
        }
    }
}

/// Decides whether Core and the agent can each keep their own runtime. Uses the launch
/// planner's own estimates: both models must fit in 60% of system RAM, and GPU-offloaded
/// weights must fit the planner's VRAM budget together. A partial offload means one model
/// alone already fills the GPU budget, so it never shares the GPU with another runtime.
pub fn decide_placement(
    mode: RuntimeMode,
    core: Option<Footprint>,
    agent: Footprint,
    total_ram_bytes: u64,
    vram_budget_bytes: Option<u64>,
) -> PlacementDecision {
    if mode == RuntimeMode::Shared {
        return PlacementDecision::new(
            Placement::Shared,
            false,
            "Shared runtime selected in Agent Setup.",
        );
    }
    let Some(core) = core else {
        return PlacementDecision::new(
            if mode == RuntimeMode::Dedicated {
                Placement::Dedicated
            } else {
                Placement::Shared
            },
            false,
            "OpenMindAI Core is not installed, so the agent does not compete with it.",
        );
    };
    let ram_ok =
        total_ram_bytes > 0 && core.bytes.saturating_add(agent.bytes) <= total_ram_bytes / 5 * 3;
    let gpu_ok = match (core.gpu, agent.gpu) {
        (GpuUse::Partial, _) | (_, GpuUse::Partial) => false,
        (GpuUse::None, GpuUse::None) => true,
        _ => {
            let on_gpu = |footprint: Footprint| {
                if footprint.gpu == GpuUse::Full {
                    footprint.bytes
                } else {
                    0
                }
            };
            vram_budget_bytes
                .is_some_and(|budget| on_gpu(core).saturating_add(on_gpu(agent)) <= budget)
        }
    };

    match mode {
        RuntimeMode::Automatic if ram_ok && gpu_ok => PlacementDecision::new(
            Placement::Dedicated,
            false,
            "Core and OpenAgent both fit in memory, so each keeps its own runtime.",
        ),
        RuntimeMode::Automatic if !ram_ok => PlacementDecision::new(
            Placement::Shared,
            false,
            "Not enough system memory to keep Core and OpenAgent loaded together, so they share one runtime.",
        ),
        RuntimeMode::Automatic => PlacementDecision::new(
            Placement::Shared,
            false,
            "Core and OpenAgent do not both fit in GPU memory, so they share one runtime.",
        ),
        _ if ram_ok && gpu_ok => PlacementDecision::new(
            Placement::Dedicated,
            false,
            "Dedicated runtime selected in Agent Setup.",
        ),
        _ if ram_ok => PlacementDecision::new(
            Placement::Dedicated,
            true,
            "Dedicated runtime selected. GPU memory stays with Core, so OpenAgent runs on the CPU in its own runtime.",
        ),
        _ => PlacementDecision::new(
            Placement::Shared,
            false,
            "Dedicated runtime selected, but there is not enough system memory for two runtimes, so the shared runtime is used.",
        ),
    }
}

/// Free memory a dedicated agent runtime must leave untouched for the rest of the system.
const DEDICATED_HEADROOM_BYTES: u64 = 1024 * 1024 * 1024;

/// `decide_placement` judges against total RAM, which says nothing about what other
/// software uses right now. Before starting a second runtime, require enough memory to be
/// available at this moment; otherwise share the runtime instead of failing the load.
/// An agent runtime that is already resident needs no new memory.
pub fn confirm_available_memory(
    decision: PlacementDecision,
    agent_bytes: u64,
    available_ram_bytes: Option<u64>,
    agent_already_resident: bool,
) -> PlacementDecision {
    if decision.placement != Placement::Dedicated || agent_already_resident {
        return decision;
    }
    let needed = agent_bytes.saturating_add(DEDICATED_HEADROOM_BYTES);
    match available_ram_bytes {
        Some(available) if available >= needed => decision,
        _ => PlacementDecision::new(
            Placement::Shared,
            false,
            "A dedicated runtime would fit this computer, but not enough memory is free right now, so the shared runtime is used.",
        ),
    }
}

// ---------------------------------------------------------------------------
// Real runtime
// ---------------------------------------------------------------------------

pub struct AgentRuntimeContext {
    pub coding_enabled: bool,
    pub mode: RuntimeMode,
    pub plan: Option<AgentRuntimePlan>,
}

#[derive(Clone)]
pub struct AgentRuntimePlan {
    pub model: ModelRecord,
    pub config: ModelLaunchConfig,
    pub decision: PlacementDecision,
}

/// Resolves Agent Setup into a model, launch configuration and runtime placement.
pub fn plan_agent_runtime(app: &AppHandle) -> Result<AgentRuntimeContext, AppError> {
    let state = app.state::<AppState>();
    let (preferences, models) = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        (
            SettingsRepository::new(&db).get_preferences()?,
            ModelRegistry::new(&db, &state.root).list_models()?,
        )
    };
    let mode = RuntimeMode::parse(&preferences.openagent_runtime_mode);
    let Some(model) = local_agent::configured_openagent_model(&state)? else {
        return Ok(AgentRuntimeContext {
            coding_enabled: preferences.coding_enabled,
            mode,
            plan: None,
        });
    };
    let hardware = HardwareProfiler::for_inference(&state.hardware);
    let mut config = local_agent::openagent_launch_config(
        &model,
        &hardware,
        &preferences,
        allocate_local_port()?,
    );
    let core = models.iter().find(|candidate| {
        candidate.enabled
            && candidate.id != model.id
            && candidate.source_repository.as_deref() == Some(warm_start::CORE_REPOSITORY)
    });
    let core_plan = core.map(|core| ModelLaunchPlanner::plan(core, &hardware, 0));
    let decision = decide_placement(
        mode,
        core_plan
            .as_ref()
            .map(|plan| Footprint::from_config(plan.estimated_model_bytes, &plan.config)),
        Footprint::from_config(model.size_bytes.max(0) as u64, &config),
        hardware.memory.total_bytes,
        core_plan
            .as_ref()
            .and_then(|plan| plan.dedicated_vram_budget_bytes),
    );
    let agent_resident = {
        let mut agent_runtime = state
            .agent_runtime
            .lock()
            .map_err(|_| AppError::internal("runtime lock poisoned"))?;
        let expected = agent_runtime.resolved_model_path(&config.model_path)?;
        agent_runtime.loaded_model_path() == Some(expected.as_str())
            && agent_runtime.is_process_alive()
    };
    let decision = confirm_available_memory(
        decision,
        Footprint::from_config(model.size_bytes.max(0) as u64, &config).bytes,
        available_memory_bytes(),
        agent_resident,
    );
    if decision.agent_on_cpu {
        config.gpu_layers = 0;
        config.backend = BackendKind::Cpu;
    }
    Ok(AgentRuntimeContext {
        coding_enabled: preferences.coding_enabled,
        mode,
        plan: Some(AgentRuntimePlan {
            model,
            config,
            decision,
        }),
    })
}

fn available_memory_bytes() -> Option<u64> {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    Some(system.available_memory()).filter(|bytes| *bytes > 0)
}

fn runtime_for(state: &AppState, placement: Placement) -> &std::sync::Mutex<LlamaRuntimeManager> {
    match placement {
        Placement::Shared => &state.runtime,
        Placement::Dedicated => &state.agent_runtime,
    }
}

struct RuntimeLauncher {
    app: AppHandle,
    placement: Placement,
    config: ModelLaunchConfig,
    expected_model_path: String,
    /// Real requests lease the model under the same lock that sees it ready, so no other
    /// request can swap it out in between. Warm-ups leave the claim window for the request
    /// that follows.
    take_lease: bool,
    lease: std::sync::Mutex<Option<ModelLease>>,
}

impl RuntimeLauncher {
    fn store_lease(&self, lease: Option<ModelLease>) {
        if let (Some(lease), Ok(mut slot)) = (lease, self.lease.lock()) {
            *slot = Some(lease);
        }
    }
}

impl RuntimeLauncher {
    async fn with_runtime<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut LlamaRuntimeManager, &AppState) -> T + Send + 'static,
    ) -> Result<T, AppError> {
        let app = self.app.clone();
        let placement = self.placement;
        tokio::task::spawn_blocking(move || {
            let state = app.state::<AppState>();
            let mut runtime = runtime_for(&state, placement)
                .lock()
                .map_err(|_| AppError::internal("runtime lock poisoned"))?;
            Ok(work(&mut runtime, &state))
        })
        .await
        .map_err(|error| AppError::internal(error.to_string()))?
    }
}

impl AgentLauncher for RuntimeLauncher {
    async fn launch(&self) -> Result<Launch, AppError> {
        let config = self.config.clone();
        let placement = self.placement;
        let take_lease = self.take_lease;
        let (launch, lease) = self
            .with_runtime(move |runtime, state| {
                let hardware = HardwareProfiler::for_inference(&state.hardware);
                let launch = runtime.launch_model_server(&hardware, &config)?;
                if placement == Placement::Shared && matches!(launch, ModelLaunch::Loading) {
                    // Core is no longer resident in the shared runtime.
                    state.warm_start.mark_runtime_stopped();
                }
                let lease = (take_lease && matches!(launch, ModelLaunch::Resident(_)))
                    .then(|| runtime.lease());
                Ok::<_, AppError>((launch, lease))
            })
            .await??;
        self.store_lease(lease);
        Ok(match launch {
            ModelLaunch::Resident(status) => Launch::Ready(status.endpoint.ok_or_else(|| {
                AppError::InferenceServerUnavailable("runtime endpoint missing".to_string())
            })?),
            ModelLaunch::Loading => Launch::Started,
            ModelLaunch::Busy => Launch::Busy,
        })
    }

    async fn probe(&self) -> ModelProbe {
        let expected = self.expected_model_path.clone();
        let take_lease = self.take_lease;
        let result = self
            .with_runtime(move |runtime, _| {
                let probe = runtime.probe_model_server(&expected);
                let lease =
                    (take_lease && matches!(probe, ModelProbe::Ready(_))).then(|| runtime.lease());
                (probe, lease)
            })
            .await;
        match result {
            Ok((probe, lease)) => {
                self.store_lease(lease);
                probe
            }
            Err(error) => ModelProbe::Failed(error.to_string()),
        }
    }

    async fn abort(&self) {
        let _ = self.with_runtime(|runtime, _| runtime.stop()).await;
    }
}

/// Unloads the agent's runtime after it produced corrupted output, so the next request
/// starts a fresh server. Only stops the runtime if it still holds the agent model.
pub async fn unload_corrupted_agent(app: &AppHandle, plan: &AgentRuntimePlan) {
    let app = app.clone();
    let plan = plan.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let Ok(mut runtime) = runtime_for(&state, plan.decision.placement).lock() else {
            return;
        };
        let Ok(expected) = runtime.resolved_model_path(&plan.config.model_path) else {
            return;
        };
        if runtime.loaded_model_path() == Some(expected.as_str()) {
            let _ = runtime.stop();
            tracing::warn!(
                "coding agent produced degenerate output; runtime unloaded for a fresh load"
            );
        }
    })
    .await;
}

/// Stops the dedicated agent llama-server if it is running. Returns whether one was stopped.
fn stop_dedicated_runtime(state: &AppState) -> bool {
    let Ok(mut runtime) = state.agent_runtime.lock() else {
        return false;
    };
    if !runtime.has_process() {
        return false;
    }
    runtime.stop().is_ok()
}

/// Loads the Agent Setup model where [`decide_placement`] put it and returns its endpoint.
/// Waits up to [`AGENT_STARTUP_TIMEOUT`]; concurrent callers share one startup.
/// The returned lease keeps the agent model resident while the caller uses it; drop it when
/// the request (or OpenAgent run) is finished.
pub async fn acquire_agent_runtime(
    app: &AppHandle,
    plan: &AgentRuntimePlan,
) -> Result<(String, ModelLease), AppError> {
    let (endpoint, lease) = load_agent_runtime(app, plan, true).await?;
    let lease = lease.ok_or_else(|| {
        AppError::ModelLoadFailed(
            "the agent model was replaced before it could be used".to_string(),
        )
    })?;
    Ok((endpoint, lease))
}

async fn load_agent_runtime(
    app: &AppHandle,
    plan: &AgentRuntimePlan,
    take_lease: bool,
) -> Result<(String, Option<ModelLease>), AppError> {
    let state = app.state::<AppState>();
    if plan.decision.placement == Placement::Shared {
        // Switching back from a dedicated runtime frees its memory.
        let app = app.clone();
        tokio::task::spawn_blocking(move || {
            stop_dedicated_runtime(&app.state::<AppState>());
        })
        .await
        .map_err(|error| AppError::internal(error.to_string()))?;
    }
    let expected_model_path = runtime_for(&state, plan.decision.placement)
        .lock()
        .map_err(|_| AppError::internal("runtime lock poisoned"))?
        .resolved_model_path(&plan.config.model_path)?;
    let launcher = RuntimeLauncher {
        app: app.clone(),
        placement: plan.decision.placement,
        config: plan.config.clone(),
        expected_model_path,
        take_lease,
        lease: std::sync::Mutex::new(None),
    };
    let endpoint = state
        .agent_startup
        .acquire(&plan.model.id, &launcher, AGENT_STARTUP_TIMEOUT)
        .await?;
    let lease = launcher.lease.lock().ok().and_then(|mut slot| slot.take());
    Ok((endpoint, lease))
}

/// Starts loading the agent in the background so callers can poll its status.
pub fn start_in_background(app: &AppHandle, plan: AgentRuntimePlan) {
    app.state::<AppState>()
        .agent_startup
        .mark_requested(&plan.model.id);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = load_agent_runtime(&app, &plan, false).await {
            tracing::warn!(%error, "coding agent background start failed");
        }
    });
}

/// Unloads an idle dedicated agent runtime when system memory runs low, mirroring the
/// Core idle-trim rule in `warm_start`.
pub fn spawn_idle_monitor(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(warm_start::MEMORY_CHECK_INTERVAL).await;
            let state = app.state::<AppState>();
            let idle = state
                .agent_startup
                .idle_for()
                .is_some_and(|idle| idle >= warm_start::IDLE_BEFORE_MEMORY_TRIM);
            if !idle || !state.active_generations.is_idle() {
                continue;
            }
            if warm_start::low_system_memory().is_none() {
                continue;
            }
            let stop_app = app.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if stop_dedicated_runtime(&stop_app.state::<AppState>()) {
                    tracing::info!(
                        "idle dedicated agent runtime unloaded because system memory is low"
                    );
                }
            })
            .await;
        }
    });
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelInfo {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub family: Option<String>,
    pub repository: Option<String>,
    pub quantization: Option<String>,
    pub path: String,
}

impl AgentModelInfo {
    pub fn from_model(model: &ModelRecord) -> Self {
        Self {
            id: model.id.clone(),
            name: model.name.clone(),
            display_name: display_name(model),
            family: model.family.clone(),
            repository: model.source_repository.clone(),
            quantization: model.quantization.clone(),
            path: model.path.clone(),
        }
    }
}

/// "nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF" -> "NVIDIA Nemotron 3 Nano 4B".
pub fn display_name(model: &ModelRecord) -> String {
    model
        .source_repository
        .as_deref()
        .and_then(|repository| repository.rsplit('/').next())
        .map(|name| name.trim_end_matches("-GGUF").replace('-', " "))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| model.name.clone())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentStatus {
    pub state: AgentPhase,
    pub coding_enabled: bool,
    pub model: Option<AgentModelInfo>,
    pub configured_context: Option<u32>,
    pub parallel_workers: Option<u32>,
    /// Tokens available to one request (one llama-server slot).
    pub effective_context: Option<i64>,
    /// "runtime" when llama-server reported it, "estimate" when computed from settings.
    pub effective_context_source: Option<&'static str>,
    pub gpu_layers: Option<i32>,
    pub runtime_mode: RuntimeMode,
    pub runtime_placement: Option<Placement>,
    pub placement_reason: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub last_startup_ms: Option<u64>,
    pub error: Option<String>,
    /// Model currently resident in the shared runtime when it is not the agent.
    pub shared_runtime_model: Option<String>,
    pub message: Option<String>,
}

/// Per-request context: the runtime-reported slot size when available, otherwise the
/// configured context divided across parallel slots.
pub fn effective_context(
    reported: Option<i64>,
    configured: u32,
    parallel: u32,
) -> (i64, &'static str) {
    match reported.filter(|value| *value > 0) {
        Some(value) => (value, "runtime"),
        None => (i64::from(configured / parallel.max(1)), "estimate"),
    }
}

pub async fn slot_context_size(http: &reqwest::Client, endpoint: &str) -> Option<i64> {
    let props: Value = http
        .get(format!("{}/props", endpoint.trim_end_matches('/')))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    props
        .pointer("/default_generation_settings/n_ctx")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
}

pub async fn agent_status(app: &AppHandle) -> Result<CodingAgentStatus, AppError> {
    let context = plan_agent_runtime(app)?;
    let state = app.state::<AppState>();
    let mut status = CodingAgentStatus {
        state: AgentPhase::NotInstalled,
        coding_enabled: context.coding_enabled,
        model: None,
        configured_context: None,
        parallel_workers: None,
        effective_context: None,
        effective_context_source: None,
        gpu_layers: None,
        runtime_mode: context.mode,
        runtime_placement: None,
        placement_reason: None,
        elapsed_ms: None,
        last_startup_ms: None,
        error: None,
        shared_runtime_model: None,
        message: None,
    };
    let Some(plan) = context.plan else {
        status.state = if context.coding_enabled {
            AgentPhase::NotInstalled
        } else {
            AgentPhase::Disabled
        };
        status.message = Some(NO_AGENT_MESSAGE.to_string());
        return Ok(status);
    };
    status.model = Some(AgentModelInfo::from_model(&plan.model));
    status.configured_context = Some(plan.config.context_size);
    status.parallel_workers = Some(plan.config.parallelism);
    status.gpu_layers = Some(plan.config.gpu_layers);
    status.runtime_placement = Some(plan.decision.placement);
    status.placement_reason = Some(plan.decision.reason.clone());

    let view = state.agent_startup.view();
    let view_is_agent = view.model_key.as_deref() == Some(plan.model.id.as_str());
    status.last_startup_ms = view
        .last_startup
        .filter(|_| view_is_agent)
        .map(|duration| duration.as_millis() as u64);

    let (resident_endpoint, other_model) = {
        let mut runtime = runtime_for(&state, plan.decision.placement)
            .lock()
            .map_err(|_| AppError::internal("runtime lock poisoned"))?;
        let expected = runtime.resolved_model_path(&plan.config.model_path)?;
        let loaded = runtime.loaded_model_path().map(str::to_string);
        if loaded.as_deref() == Some(expected.as_str())
            && runtime.state() == ServerState::Ready
            && runtime.is_process_alive()
        {
            (runtime.endpoint(), None)
        } else {
            (
                None,
                loaded.filter(|path| path != &expected).map(|path| {
                    Path::new(&path)
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or(path)
                }),
            )
        }
    };

    let loading = view_is_agent
        && matches!(
            view.phase,
            Some(AgentPhase::Starting | AgentPhase::LoadingModel)
        );
    status.state = if !context.coding_enabled {
        AgentPhase::Disabled
    } else if loading {
        status.elapsed_ms = view.elapsed.map(|elapsed| elapsed.as_millis() as u64);
        view.phase.unwrap_or(AgentPhase::Starting)
    } else if resident_endpoint.is_some() {
        AgentPhase::Ready
    } else if view_is_agent && view.phase == Some(AgentPhase::Error) {
        status.error = view.error.clone();
        AgentPhase::Error
    } else {
        AgentPhase::Stopped
    };
    if plan.decision.placement == Placement::Shared && status.state != AgentPhase::Ready {
        status.shared_runtime_model = other_model;
    }
    if status.state == AgentPhase::Disabled {
        status.message = Some(DISABLED_MESSAGE.to_string());
    }

    let reported = match &resident_endpoint {
        Some(endpoint) => slot_context_size(&state.http, endpoint).await,
        None => None,
    };
    let (effective, source) =
        effective_context(reported, plan.config.context_size, plan.config.parallelism);
    status.effective_context = Some(effective);
    status.effective_context_source = Some(source);
    Ok(status)
}

pub const NO_AGENT_MESSAGE: &str = "No OpenAgent coding model is installed. Download an OpenAgent (NVIDIA Nemotron) model in OpenMindAI Settings -> Agent Setup.";
pub const DISABLED_MESSAGE: &str = "Coding Workspace is disabled in OpenMindAI Settings -> Agent Setup, so VS Code and OpenAgent coding requests are unavailable. Enable it to use the coding agent.";

#[tauri::command]
pub async fn coding_agent_status(app: AppHandle) -> Result<CodingAgentStatus, AppError> {
    agent_status(&app).await
}

/// Starts loading the coding agent without waiting, then reports its status.
#[tauri::command]
pub async fn start_coding_agent(app: AppHandle) -> Result<CodingAgentStatus, AppError> {
    let context = plan_agent_runtime(&app)?;
    if let Some(plan) = context.plan.filter(|_| context.coding_enabled) {
        start_in_background(&app, plan);
    }
    agent_status(&app).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize},
        Arc,
    };

    /// Fake runtime: becomes healthy `load_time` after the first launch.
    struct FakeLauncher {
        busy_until: Option<Duration>,
        created: Instant,
        load_time: Duration,
        fail_after: Option<Duration>,
        loaded_at: std::sync::Mutex<Option<Instant>>,
        spawns: AtomicUsize,
        aborted: AtomicBool,
    }

    impl FakeLauncher {
        fn new(load_time: Duration) -> Arc<Self> {
            Arc::new(Self {
                busy_until: None,
                created: Instant::now(),
                load_time,
                fail_after: None,
                loaded_at: std::sync::Mutex::new(None),
                spawns: AtomicUsize::new(0),
                aborted: AtomicBool::new(false),
            })
        }

        fn failing(after: Duration) -> Arc<Self> {
            Arc::new(Self {
                busy_until: None,
                created: Instant::now(),
                load_time: Duration::from_secs(3600),
                fail_after: Some(after),
                loaded_at: std::sync::Mutex::new(None),
                spawns: AtomicUsize::new(0),
                aborted: AtomicBool::new(false),
            })
        }

        fn resident() -> Arc<Self> {
            let launcher = Self::new(Duration::ZERO);
            *launcher.loaded_at.lock().unwrap() = Some(Instant::now());
            launcher
        }

        fn is_healthy(&self) -> bool {
            self.loaded_at
                .lock()
                .unwrap()
                .is_some_and(|at| at.elapsed() >= self.load_time)
        }
    }

    impl AgentLauncher for Arc<FakeLauncher> {
        async fn launch(&self) -> Result<Launch, AppError> {
            if self
                .busy_until
                .is_some_and(|until| self.created.elapsed() < until)
            {
                return Ok(Launch::Busy);
            }
            let mut loaded_at = self.loaded_at.lock().unwrap();
            if loaded_at.is_some() {
                drop(loaded_at);
                return Ok(if self.is_healthy() {
                    Launch::Ready("http://127.0.0.1:1".to_string())
                } else {
                    Launch::Started
                });
            }
            *loaded_at = Some(Instant::now());
            self.spawns.fetch_add(1, Ordering::SeqCst);
            Ok(Launch::Started)
        }

        async fn probe(&self) -> ModelProbe {
            let started = self
                .loaded_at
                .lock()
                .unwrap()
                .expect("probed before launch");
            if let Some(after) = self.fail_after {
                if started.elapsed() >= after {
                    return ModelProbe::Failed("llama-server exited".to_string());
                }
            }
            if self.is_healthy() {
                ModelProbe::Ready("http://127.0.0.1:1".to_string())
            } else {
                ModelProbe::Loading
            }
        }

        async fn abort(&self) {
            self.aborted.store(true, Ordering::SeqCst);
            *self.loaded_at.lock().unwrap() = None;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn ready_agent_is_reused_without_loading() {
        let startup = AgentStartup::default();
        let launcher = FakeLauncher::resident();
        let started = Instant::now();
        let endpoint = startup
            .acquire("agent", &launcher, AGENT_STARTUP_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(endpoint, "http://127.0.0.1:1");
        assert_eq!(launcher.spawns.load(Ordering::SeqCst), 0);
        assert_eq!(started.elapsed(), Duration::ZERO);
        assert_eq!(startup.view().phase, Some(AgentPhase::Ready));
    }

    #[tokio::test(start_paused = true)]
    async fn cold_start_reports_loading_then_ready() {
        let startup = Arc::new(AgentStartup::default());
        let launcher = FakeLauncher::new(Duration::from_secs(20));
        let task = {
            let startup = startup.clone();
            let launcher = launcher.clone();
            tokio::spawn(async move {
                startup
                    .acquire("agent", &launcher, AGENT_STARTUP_TIMEOUT)
                    .await
            })
        };
        tokio::time::sleep(Duration::from_secs(5)).await;
        let view = startup.view();
        assert_eq!(view.phase, Some(AgentPhase::LoadingModel));
        assert!(view.elapsed.unwrap() >= Duration::from_secs(5));
        task.await.unwrap().unwrap();
        let view = startup.view();
        assert_eq!(view.phase, Some(AgentPhase::Ready));
        assert!(view.last_startup.unwrap() >= Duration::from_secs(20));
        assert_eq!(launcher.spawns.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn startup_longer_than_the_old_120s_limit_succeeds() {
        let startup = AgentStartup::default();
        // The observed Nemotron cold start.
        let launcher = FakeLauncher::new(Duration::from_secs(155));
        let started = Instant::now();
        startup
            .acquire("agent", &launcher, AGENT_STARTUP_TIMEOUT)
            .await
            .unwrap();
        assert!(started.elapsed() >= Duration::from_secs(155));
        assert!(!launcher.aborted.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_requests_share_one_startup() {
        let startup = Arc::new(AgentStartup::default());
        let launcher = FakeLauncher::new(Duration::from_secs(150));
        let tasks: Vec<_> = (0..4)
            .map(|_| {
                let startup = startup.clone();
                let launcher = launcher.clone();
                tokio::spawn(async move {
                    startup
                        .acquire("agent", &launcher, AGENT_STARTUP_TIMEOUT)
                        .await
                })
            })
            .collect();
        for task in tasks {
            assert_eq!(task.await.unwrap().unwrap(), "http://127.0.0.1:1");
        }
        assert_eq!(launcher.spawns.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn startup_failure_is_reported_and_shared_with_waiters() {
        let startup = Arc::new(AgentStartup::default());
        let launcher = FakeLauncher::failing(Duration::from_secs(30));
        let tasks: Vec<_> = (0..3)
            .map(|_| {
                let startup = startup.clone();
                let launcher = launcher.clone();
                tokio::spawn(async move {
                    startup
                        .acquire("agent", &launcher, AGENT_STARTUP_TIMEOUT)
                        .await
                })
            })
            .collect();
        for task in tasks {
            let error = task.await.unwrap().unwrap_err().to_string();
            assert!(error.contains("llama-server exited"), "{error}");
        }
        assert_eq!(launcher.spawns.load(Ordering::SeqCst), 1);
        let view = startup.view();
        assert_eq!(view.phase, Some(AgentPhase::Error));
        assert!(view.error.unwrap().contains("exited"));
    }

    #[tokio::test(start_paused = true)]
    async fn startup_times_out_at_the_configured_upper_bound() {
        let startup = AgentStartup::default();
        let launcher = FakeLauncher::new(Duration::from_secs(10_000));
        let started = Instant::now();
        let error = startup
            .acquire("agent", &launcher, AGENT_STARTUP_TIMEOUT)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("300 seconds"), "{error}");
        assert!(started.elapsed() >= AGENT_STARTUP_TIMEOUT);
        assert!(started.elapsed() < AGENT_STARTUP_TIMEOUT + Duration::from_secs(2));
        assert!(launcher.aborted.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn request_right_after_a_delayed_startup_is_immediate() {
        let startup = AgentStartup::default();
        let launcher = FakeLauncher::new(Duration::from_secs(200));
        startup
            .acquire("agent", &launcher, AGENT_STARTUP_TIMEOUT)
            .await
            .unwrap();
        let second = Instant::now();
        startup
            .acquire("agent", &launcher, AGENT_STARTUP_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(second.elapsed(), Duration::ZERO);
        assert_eq!(launcher.spawns.load(Ordering::SeqCst), 1);
        // The last cold-start duration stays visible after the fast reuse.
        assert!(startup.view().last_startup.unwrap() >= Duration::from_secs(200));
    }

    #[tokio::test(start_paused = true)]
    async fn waits_for_another_model_to_finish_loading_before_switching() {
        let startup = AgentStartup::default();
        let launcher = Arc::new(FakeLauncher {
            busy_until: Some(Duration::from_secs(90)),
            created: Instant::now(),
            load_time: Duration::from_secs(60),
            fail_after: None,
            loaded_at: std::sync::Mutex::new(None),
            spawns: AtomicUsize::new(0),
            aborted: AtomicBool::new(false),
        });
        let started = Instant::now();
        startup
            .acquire("agent", &launcher, AGENT_STARTUP_TIMEOUT)
            .await
            .unwrap();
        // 90 s waiting for Core's load, then the agent's own 60 s load: one spawn, no abort.
        assert!(started.elapsed() >= Duration::from_secs(150));
        assert_eq!(launcher.spawns.load(Ordering::SeqCst), 1);
        assert!(!launcher.aborted.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn mark_requested_shows_starting_before_the_gate() {
        let startup = AgentStartup::default();
        startup.mark_requested("agent");
        tokio::time::sleep(Duration::from_secs(3)).await;
        let view = startup.view();
        assert_eq!(view.phase, Some(AgentPhase::Starting));
        assert!(view.elapsed.unwrap() >= Duration::from_secs(3));
    }

    const GIB: u64 = 1024 * 1024 * 1024;

    fn footprint(gib: u64, gpu: GpuUse) -> Footprint {
        Footprint {
            bytes: gib * GIB,
            gpu,
        }
    }

    #[test]
    fn automatic_uses_dedicated_runtimes_when_both_fit() {
        let decision = decide_placement(
            RuntimeMode::Automatic,
            Some(footprint(4, GpuUse::Full)),
            footprint(4, GpuUse::Full),
            32 * GIB,
            Some(20 * GIB),
        );
        assert_eq!(decision.placement, Placement::Dedicated);
        assert!(!decision.agent_on_cpu);
    }

    #[test]
    fn automatic_shares_on_a_4gb_gpu_with_partial_offload() {
        // This machine: RX 580 4 GB, Core and OpenAgent Lite each partially offloaded.
        let decision = decide_placement(
            RuntimeMode::Automatic,
            Some(footprint(4, GpuUse::Partial)),
            footprint(4, GpuUse::Partial),
            16 * GIB,
            Some(2 * GIB),
        );
        assert_eq!(decision.placement, Placement::Shared);
        assert!(decision.reason.contains("GPU memory"));
    }

    #[test]
    fn automatic_shares_when_ram_is_short_or_unknown() {
        let both_cpu = (footprint(6, GpuUse::None), footprint(6, GpuUse::None));
        let tight = decide_placement(
            RuntimeMode::Automatic,
            Some(both_cpu.0),
            both_cpu.1,
            16 * GIB,
            None,
        );
        assert_eq!(tight.placement, Placement::Shared);
        assert!(tight.reason.contains("system memory"));
        let unknown = decide_placement(
            RuntimeMode::Automatic,
            Some(both_cpu.0),
            both_cpu.1,
            0,
            None,
        );
        assert_eq!(unknown.placement, Placement::Shared);
        let roomy = decide_placement(
            RuntimeMode::Automatic,
            Some(both_cpu.0),
            both_cpu.1,
            64 * GIB,
            None,
        );
        assert_eq!(roomy.placement, Placement::Dedicated);
    }

    #[test]
    fn full_offloads_must_fit_the_vram_budget_together() {
        let decision = decide_placement(
            RuntimeMode::Automatic,
            Some(footprint(5, GpuUse::Full)),
            footprint(5, GpuUse::Full),
            64 * GIB,
            Some(8 * GIB),
        );
        assert_eq!(decision.placement, Placement::Shared);
    }

    #[test]
    fn forced_dedicated_moves_the_agent_to_cpu_when_the_gpu_is_full() {
        let decision = decide_placement(
            RuntimeMode::Dedicated,
            Some(footprint(4, GpuUse::Partial)),
            footprint(4, GpuUse::Partial),
            16 * GIB,
            Some(2 * GIB),
        );
        assert_eq!(decision.placement, Placement::Dedicated);
        assert!(decision.agent_on_cpu);
    }

    #[test]
    fn forced_dedicated_falls_back_to_shared_without_ram() {
        let decision = decide_placement(
            RuntimeMode::Dedicated,
            Some(footprint(8, GpuUse::None)),
            footprint(8, GpuUse::None),
            16 * GIB,
            None,
        );
        assert_eq!(decision.placement, Placement::Shared);
        assert!(decision.reason.contains("not enough system memory"));
    }

    #[test]
    fn dedicated_needs_memory_that_is_free_right_now() {
        let dedicated = PlacementDecision::new(Placement::Dedicated, false, "fits");
        // Enough free memory: keep the dedicated runtime.
        let kept = confirm_available_memory(dedicated.clone(), 4 * GIB, Some(6 * GIB), false);
        assert_eq!(kept.placement, Placement::Dedicated);
        // Machine fits on paper but is busy now: share instead of failing the load.
        let busy = confirm_available_memory(dedicated.clone(), 4 * GIB, Some(4 * GIB), false);
        assert_eq!(busy.placement, Placement::Shared);
        assert!(busy.reason.contains("not enough memory is free right now"));
        // Unknown availability is treated as not enough.
        assert_eq!(
            confirm_available_memory(dedicated.clone(), 4 * GIB, None, false).placement,
            Placement::Shared
        );
        // Already resident: no new memory needed, keep it.
        assert_eq!(
            confirm_available_memory(dedicated, 4 * GIB, Some(0), true).placement,
            Placement::Dedicated
        );
        // Shared decisions are never upgraded.
        let shared = PlacementDecision::new(Placement::Shared, false, "gpu");
        assert_eq!(
            confirm_available_memory(shared, 1, Some(64 * GIB), false).placement,
            Placement::Shared
        );
    }

    #[test]
    fn shared_mode_and_missing_core() {
        let agent = footprint(4, GpuUse::Partial);
        assert_eq!(
            decide_placement(RuntimeMode::Shared, Some(agent), agent, 64 * GIB, None).placement,
            Placement::Shared
        );
        assert_eq!(
            decide_placement(RuntimeMode::Automatic, None, agent, 64 * GIB, None).placement,
            Placement::Shared
        );
        assert_eq!(RuntimeMode::parse("dedicated"), RuntimeMode::Dedicated);
        assert_eq!(RuntimeMode::parse("bogus"), RuntimeMode::Automatic);
    }

    #[test]
    fn effective_context_prefers_runtime_report() {
        assert_eq!(effective_context(None, 8192, 1), (8192, "estimate"));
        assert_eq!(effective_context(None, 8192, 2), (4096, "estimate"));
        assert_eq!(effective_context(None, 16384, 2), (8192, "estimate"));
        // A unified KV cache gives each slot the full context, unlike naive division.
        assert_eq!(effective_context(Some(8192), 8192, 2), (8192, "runtime"));
        assert_eq!(effective_context(Some(0), 8192, 0), (8192, "estimate"));
    }

    #[test]
    fn display_name_comes_from_the_repository() {
        let mut model = crate::model_registry::ModelRecord {
            id: "lite".to_string(),
            name: "OpenAgent Lite".to_string(),
            family: Some("nemotron_h".to_string()),
            path: "models/x.gguf".to_string(),
            format: "gguf".to_string(),
            quantization: Some("Q4_K_M".to_string()),
            size_bytes: 0,
            capabilities: "[]".to_string(),
            context_length: None,
            preferred_backend: None,
            enabled: true,
            source_repository: Some("nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF".to_string()),
            verification: None,
            state: crate::model_registry::ModelLifecycleState::Ready,
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert_eq!(display_name(&model), "NVIDIA Nemotron 3 Nano 4B");
        model.source_repository = None;
        assert_eq!(display_name(&model), "OpenAgent Lite");
    }
}

/// Measures model reloads on the real runtime. Run manually:
/// `OPENMINDAI_REAL_ROOT="G:\portable ai" cargo test --lib real_runtime_switching -- --ignored --nocapture`
/// Uses CPU-only launches so it does not compete with a running desktop app for the GPU.
#[cfg(test)]
mod real_runtime {
    use super::*;
    use crate::{hardware::HardwareProfile, portable_root::PortableRootManager};
    use serde_json::json;
    use std::{
        path::PathBuf,
        sync::{atomic::AtomicUsize, Arc, Mutex},
    };

    const CORE: &str = "models/llm/qwen/qwen3-4b/Qwen3-4B-Q4_K_M.gguf";
    const AGENT: &str = "models/llm/openmind/agent-lite/NVIDIA-Nemotron3-Nano-4B-Q4_K_M.gguf";

    struct RealLauncher {
        runtime: Arc<Mutex<LlamaRuntimeManager>>,
        hardware: Arc<HardwareProfile>,
        config: ModelLaunchConfig,
        expected: String,
        spawns: Arc<AtomicUsize>,
    }

    impl AgentLauncher for RealLauncher {
        async fn launch(&self) -> Result<Launch, AppError> {
            let mut runtime = self.runtime.lock().unwrap();
            let reload = runtime.loaded_model_path() != Some(self.expected.as_str());
            let launch = runtime.launch_model_server(&self.hardware, &self.config)?;
            if reload && matches!(launch, ModelLaunch::Loading) {
                self.spawns.fetch_add(1, Ordering::SeqCst);
            }
            if matches!(launch, ModelLaunch::Resident(_)) {
                // One request used the model and finished.
                drop(runtime.lease());
            }
            Ok(match launch {
                ModelLaunch::Resident(status) => Launch::Ready(status.endpoint.unwrap_or_default()),
                ModelLaunch::Loading => Launch::Started,
                ModelLaunch::Busy => Launch::Busy,
            })
        }

        async fn probe(&self) -> ModelProbe {
            let mut runtime = self.runtime.lock().unwrap();
            let probe = runtime.probe_model_server(&self.expected);
            if matches!(probe, ModelProbe::Ready(_)) {
                drop(runtime.lease());
            }
            probe
        }

        async fn abort(&self) {
            let _ = self.runtime.lock().unwrap().stop();
        }
    }

    /// GPU placement matches what Automatic chooses on this machine (partial offload,
    /// shared runtime). CPU is the forced-Dedicated agent placement when the GPU is full.
    fn config(model_path: &str, on_gpu: bool) -> ModelLaunchConfig {
        ModelLaunchConfig {
            model_path: model_path.to_string(),
            backend: if on_gpu {
                BackendKind::Vulkan
            } else {
                BackendKind::Cpu
            },
            device: None,
            gpu_layers: if on_gpu { 32 } else { 0 },
            context_size: 4096,
            threads: 4,
            batch_size: 512,
            ubatch_size: 256,
            flash_attention: false,
            mmap: true,
            mlock: false,
            parallelism: 1,
            host: "127.0.0.1".to_string(),
            port: allocate_local_port().unwrap(),
        }
    }

    struct Harness {
        /// Forced-Dedicated placement on a GPU that cannot hold both models.
        agent_on_cpu: bool,
        hardware: Arc<HardwareProfile>,
        spawns: Arc<AtomicUsize>,
        log: Vec<Value>,
    }

    impl Harness {
        async fn request(
            &mut self,
            label: &str,
            runtime: &Arc<Mutex<LlamaRuntimeManager>>,
            startup: &AgentStartup,
            model_path: &str,
        ) {
            let expected = runtime
                .lock()
                .unwrap()
                .resolved_model_path(model_path)
                .unwrap();
            let launcher = RealLauncher {
                runtime: runtime.clone(),
                hardware: self.hardware.clone(),
                config: config(model_path, !(self.agent_on_cpu && model_path == AGENT)),
                expected,
                spawns: self.spawns.clone(),
            };
            let before = self.spawns.load(Ordering::SeqCst);
            let started = Instant::now();
            let endpoint = startup
                .acquire(model_path, &launcher, AGENT_STARTUP_TIMEOUT)
                .await
                .unwrap();
            self.log.push(json!({
                "step": label,
                "reloaded": self.spawns.load(Ordering::SeqCst) > before,
                "seconds": (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
                "endpoint": endpoint,
            }));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs OPENMINDAI_REAL_ROOT with installed Core and OpenAgent Lite models"]
    async fn real_runtime_switching() {
        let root = PathBuf::from(std::env::var("OPENMINDAI_REAL_ROOT").unwrap());
        let root = PortableRootManager::from_root(root);
        let root_for_concurrency = root.clone();
        let mut harness = Harness {
            agent_on_cpu: false,
            hardware: Arc::new(crate::hardware::HardwareProfiler::detect()),
            spawns: Arc::new(AtomicUsize::new(0)),
            log: Vec::new(),
        };

        // Shared placement: Core and the agent use one llama-server.
        let shared = Arc::new(Mutex::new(LlamaRuntimeManager::new(root.clone())));
        let (core_startup, agent_startup) = (AgentStartup::default(), AgentStartup::default());
        harness
            .request("shared: Core chat", &shared, &core_startup, CORE)
            .await;
        harness
            .request("shared: VS Code agent", &shared, &agent_startup, AGENT)
            .await;
        harness
            .request("shared: Core chat", &shared, &core_startup, CORE)
            .await;
        harness
            .request("shared: VS Code agent", &shared, &agent_startup, AGENT)
            .await;
        harness
            .request("shared: desktop OpenAgent", &shared, &agent_startup, AGENT)
            .await;
        harness
            .request("shared: VS Code agent", &shared, &agent_startup, AGENT)
            .await;
        let _ = shared.lock().unwrap().stop();

        // Dedicated placement: each keeps its own llama-server.
        let core_rt = Arc::new(Mutex::new(LlamaRuntimeManager::new(root.clone())));
        let agent_rt = Arc::new(Mutex::new(LlamaRuntimeManager::new(root)));
        let (core_startup, agent_startup) = (AgentStartup::default(), AgentStartup::default());
        harness.agent_on_cpu = true;
        harness
            .request("dedicated: Core chat", &core_rt, &core_startup, CORE)
            .await;
        harness
            .request("dedicated: VS Code agent", &agent_rt, &agent_startup, AGENT)
            .await;
        harness
            .request("dedicated: Core chat", &core_rt, &core_startup, CORE)
            .await;
        harness
            .request("dedicated: VS Code agent", &agent_rt, &agent_startup, AGENT)
            .await;
        harness
            .request(
                "dedicated: desktop OpenAgent",
                &agent_rt,
                &agent_startup,
                AGENT,
            )
            .await;
        harness
            .request("dedicated: VS Code agent", &agent_rt, &agent_startup, AGENT)
            .await;
        let _ = core_rt.lock().unwrap().stop();
        let _ = agent_rt.lock().unwrap().stop();

        // Shared runtime, Core chat and a VS Code request arriving together: the later one
        // must wait for the in-progress load (ModelLaunch::Busy) instead of killing it.
        harness.agent_on_cpu = false;
        let shared = Arc::new(Mutex::new(LlamaRuntimeManager::new(root_for_concurrency)));
        // Observer: samples the runtime every 250 ms to record the model load sequence and the
        // longest time anyone had to wait for the runtime lock (status readers do this).
        let observing = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let observer = {
            let shared = shared.clone();
            let observing = observing.clone();
            std::thread::spawn(move || {
                let mut sequence: Vec<String> = Vec::new();
                let mut longest_lock_wait = Duration::ZERO;
                while observing.load(Ordering::SeqCst) {
                    let asked = std::time::Instant::now();
                    let state = {
                        let manager = shared.lock().unwrap();
                        let model = manager
                            .loaded_model_path()
                            .map(|path| {
                                if path.contains("agent-lite") {
                                    "Agent"
                                } else {
                                    "Core"
                                }
                            })
                            .unwrap_or("none");
                        format!("{model}:{:?}", manager.state())
                    };
                    longest_lock_wait = longest_lock_wait.max(asked.elapsed());
                    if sequence.last() != Some(&state) {
                        sequence.push(state);
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                (sequence, longest_lock_wait)
            })
        };
        let core_task = {
            let shared = shared.clone();
            let hardware = harness.hardware.clone();
            tokio::task::spawn_blocking(move || {
                let started = std::time::Instant::now();
                let (status, lease) =
                    crate::runtime::ensure_model_ready(&shared, &hardware, &config(CORE, true))?;
                // Core uses its model for a short chat request before releasing it.
                std::thread::sleep(Duration::from_secs(3));
                drop(lease);
                Ok::<_, AppError>((started.elapsed(), status.endpoint))
            })
        };
        tokio::time::sleep(Duration::from_secs(2)).await;
        let agent_started = Instant::now();
        let concurrent_startup = AgentStartup::default();
        harness
            .request(
                "concurrent: VS Code agent while Core loads",
                &shared,
                &concurrent_startup,
                AGENT,
            )
            .await;
        let agent_elapsed = agent_started.elapsed();
        let (core_elapsed, core_endpoint) = core_task.await.unwrap().unwrap();
        observing.store(false, Ordering::SeqCst);
        let (sequence, longest_lock_wait) = observer.join().unwrap();
        harness.log.push(json!({
            "step": "concurrent: Core chat (started first)",
            "seconds": (core_elapsed.as_secs_f64() * 10.0).round() / 10.0,
            "endpoint": core_endpoint,
        }));
        harness.log.push(json!({
            "step": "concurrent: observed runtime sequence",
            "sequence": sequence,
            "longestLockWaitMs": longest_lock_wait.as_millis() as u64,
            "agentStatusReadable": concurrent_startup.view().phase.is_some(),
        }));
        println!("{}", serde_json::to_string_pretty(&harness.log).unwrap());
        let agent_path = shared.lock().unwrap().resolved_model_path(AGENT).unwrap();
        assert_eq!(
            shared.lock().unwrap().loaded_model_path(),
            Some(agent_path.as_str())
        );
        // The agent finished after Core's load completed, i.e. it waited instead of killing it.
        assert!(agent_elapsed > core_elapsed - Duration::from_secs(2));
        // Core finished loading (Ready) before the agent replaced it, and nothing else loaded.
        let core_ready = sequence.iter().position(|state| state == "Core:Ready");
        let agent_first = sequence
            .iter()
            .position(|state| state.starts_with("Agent:"));
        assert!(
            core_ready.is_some() && agent_first.is_some() && core_ready < agent_first,
            "{sequence:?}"
        );
        // Status readers only ever wait for short launch/stop steps (stopping a GPU server
        // can take a few seconds while the driver frees memory), never for a model load.
        assert!(
            longest_lock_wait < Duration::from_secs(15) && longest_lock_wait * 4 < core_elapsed,
            "{longest_lock_wait:?} vs Core load {core_elapsed:?}"
        );
        let _ = shared.lock().unwrap().stop();

        println!("{}", serde_json::to_string_pretty(&harness.log).unwrap());
        let reloads = |prefix: &str| {
            harness
                .log
                .iter()
                .filter(|entry| entry["step"].as_str().unwrap().starts_with(prefix))
                .filter(|entry| entry["reloaded"] == true)
                .count()
        };
        // Shared: initial Core load plus a swap on each Core/agent alternation.
        assert_eq!(reloads("shared"), 4);
        // Dedicated: only the first load of each model.
        assert_eq!(reloads("dedicated"), 2);
    }
}
