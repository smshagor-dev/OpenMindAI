use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, ToSocketAddrs},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    app_error::AppError,
    hardware::{BackendKind, GpuVendor, HardwareProfile},
    launch_planner::ModelLaunchConfig,
    portable_root::PortableRootManager,
    process_ownership,
};

/// Upper bound for one model load. Cold loads of 4B GGUF models were observed at 2.5-3.5
/// minutes on a 4 GB Vulkan GPU or under memory pressure, beyond the old 120 s limit.
pub const MODEL_LOAD_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a freshly loaded model stays reserved for the caller that requested it, so a
/// waiting request for another model cannot swap it out before that caller uses it.
pub const CLAIM_WINDOW: Duration = Duration::from_secs(15);

/// Held while a caller uses the loaded model (one chat request, one agent run). While any
/// lease is alive, requests for a different model wait instead of replacing it.
pub struct ModelLease(Arc<AtomicUsize>);

impl Drop for ModelLease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapDecision {
    /// Wait: the resident model is still loading, in use, or reserved for its requester.
    Wait,
    /// Safe to stop the resident model and load the requested one.
    Replace,
}

/// Whether a request for a *different* model may replace the resident one.
pub fn swap_decision(loading_fresh: bool, leases: usize, claim_pending: bool) -> SwapDecision {
    if loading_fresh || leases > 0 || claim_pending {
        SwapDecision::Wait
    } else {
        SwapDecision::Replace
    }
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeStatus {
    Available,
    Validating,
    Ready,
    Invalid,
    Missing,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ServerState {
    Stopped,
    Starting,
    Ready,
    LoadingModel,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeManifest {
    pub runtime_name: String,
    pub version: String,
    pub platform: String,
    pub architecture: String,
    pub backend: BackendKind,
    pub source: String,
    pub installed_at: String,
    pub binaries: RuntimeBinaries,
    pub checksum: Option<String>,
    pub status: RuntimeStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeBinaries {
    pub server: Option<String>,
    pub cli: Option<String>,
    pub bench: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeValidation {
    pub manifest: RuntimeManifest,
    pub server_exists: bool,
    pub cli_exists: bool,
    pub bench_exists: bool,
    pub version_output: Option<String>,
    pub device_output: Option<String>,
    pub usable: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInventory {
    pub runtimes: Vec<RuntimeValidation>,
    pub selected: Option<RuntimeValidation>,
    pub server_state: ServerState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlamaRuntimeStatus {
    pub available: bool,
    pub backend: Option<BackendKind>,
    pub endpoint: Option<String>,
    pub state: ServerState,
    pub selected_runtime: Option<RuntimeValidation>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSelector;

/// Result of starting (or reusing) a model server without waiting for the model to load.
pub enum ModelLaunch {
    /// The requested model is already resident and healthy.
    Resident(Box<LlamaRuntimeStatus>),
    /// A server process for the requested model exists but is still loading.
    Loading,
    /// Another model is still loading in this runtime. Replacing it mid-load would waste
    /// the work and make the two callers fight, so the caller waits and retries.
    Busy,
}

/// Readiness of the model server for one specific model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelProbe {
    Loading,
    Ready(String),
    /// The process exited or another model replaced it; the string explains which.
    Failed(String),
}

pub struct LlamaRuntimeManager {
    root: PortableRootManager,
    child: Option<Child>,
    endpoint: Option<String>,
    state: ServerState,
    loaded_model_path: Option<String>,
    active_runtime: Option<RuntimeValidation>,
    load_started: Option<Instant>,
    leases: Arc<AtomicUsize>,
    claim_until: Option<Instant>,
}

impl LlamaRuntimeManager {
    pub fn new(root: PortableRootManager) -> Self {
        Self {
            root,
            child: None,
            endpoint: None,
            state: ServerState::Stopped,
            loaded_model_path: None,
            active_runtime: None,
            load_started: None,
            leases: Arc::new(AtomicUsize::new(0)),
            claim_until: None,
        }
    }

    /// Where owned runtime processes are recorded for the startup sweep.
    pub fn ownership_registry(root: &PortableRootManager) -> PathBuf {
        process_ownership::registry_path(&root.root().join("runtimes"))
    }

    /// Spawns llama-server so it cannot outlive OpenMindAI (Job Object on Windows,
    /// parent-death signal on Linux) and records it for the startup sweep.
    fn spawn_owned(&self, command: Command) -> std::io::Result<Child> {
        let child = process_ownership::spawn_owned(command)?;
        process_ownership::record(&Self::ownership_registry(&self.root), child.id());
        Ok(child)
    }

    fn active_status(&self) -> LlamaRuntimeStatus {
        LlamaRuntimeStatus {
            available: self.active_runtime.is_some(),
            backend: self
                .active_runtime
                .as_ref()
                .map(|runtime| runtime.manifest.backend.clone()),
            endpoint: self.endpoint.clone(),
            state: self.state.clone(),
            selected_runtime: self.active_runtime.clone(),
        }
    }

    pub fn status(&self, hardware: &HardwareProfile) -> Result<LlamaRuntimeStatus, AppError> {
        let inventory = self.inventory(hardware)?;
        Ok(LlamaRuntimeStatus {
            available: inventory.selected.is_some(),
            backend: inventory
                .selected
                .as_ref()
                .map(|runtime| runtime.manifest.backend.clone()),
            endpoint: self.endpoint.clone(),
            state: self.state.clone(),
            selected_runtime: inventory.selected,
        })
    }

    /// Runtime discovery on the UI/chat path must be metadata-only. Older
    /// builds spawned each llama binary with --version and --list-devices here,
    /// adding up to 20 seconds of avoidable latency before a model could start.
    /// The real launch/health check below is the authoritative validation.
    pub fn inventory(&self, hardware: &HardwareProfile) -> Result<RuntimeInventory, AppError> {
        let manifests = RuntimeRegistry::new(&self.root).discover()?;
        let validations = manifests
            .into_iter()
            .map(|manifest| validate_manifest(&self.root, manifest))
            .collect::<Result<Vec<_>, _>>()?;
        let selected = RuntimeSelector::select(&validations, hardware);

        Ok(RuntimeInventory {
            runtimes: validations,
            selected,
            server_state: self.state.clone(),
        })
    }

    pub fn start_server(
        &mut self,
        hardware: &HardwareProfile,
    ) -> Result<LlamaRuntimeStatus, AppError> {
        if self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_none())
        {
            return self.status(hardware);
        }

        let selected = self.inventory(hardware)?.selected.ok_or_else(|| {
            AppError::RuntimeNotFound("no usable llama.cpp runtime found".to_string())
        })?;
        let server = selected.manifest.binaries.server.as_ref().ok_or_else(|| {
            AppError::RuntimeNotFound("llama-server executable missing".to_string())
        })?;
        let server_path = self.root.resolve_relative(server)?;
        let port = allocate_local_port()?;
        let endpoint = format!("http://127.0.0.1:{port}");
        let log_path = self
            .root
            .resolve_relative(format!("logs/llama-server-{}.log", Utc::now().timestamp()))?;
        let stderr_path = self.root.resolve_relative(format!(
            "logs/llama-server-{}-stderr.log",
            Utc::now().timestamp()
        ))?;
        let stdout = fs::File::create(log_path)?;
        let stderr = fs::File::create(stderr_path)?;

        self.state = ServerState::Starting;
        let mut command = Command::new(server_path);
        command
            .args([
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--no-webui",
            ])
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        hide_console_window(&mut command);
        let child = self
            .spawn_owned(command)
            .map_err(|error| AppError::RuntimeStartFailed(error.to_string()))?;

        self.child = Some(child);
        self.endpoint = Some(endpoint.clone());
        self.active_runtime = Some(selected);

        if wait_for_localhost(port, Duration::from_secs(8)) {
            self.state = ServerState::Ready;
        } else if self
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok())
            .flatten()
            .is_some()
        {
            self.state = ServerState::Failed;
            return Err(AppError::RuntimeStartFailed(
                "llama-server exited before accepting localhost connections".to_string(),
            ));
        } else {
            self.state = ServerState::Running;
        }

        Ok(self.active_status())
    }

    /// Starts the model server for `config` without waiting for the model to load. Reuses a
    /// live process that already serves the same model, so repeated calls never spawn a
    /// second load of the same model.
    pub fn launch_model_server(
        &mut self,
        hardware: &HardwareProfile,
        config: &ModelLaunchConfig,
    ) -> Result<ModelLaunch, AppError> {
        let model_path = resolve_model_path(&self.root, &config.model_path)?;
        let model_path_string = model_path.display().to_string();
        let selected = self.inventory(hardware)?.selected.ok_or_else(|| {
            AppError::RuntimeNotFound("no usable llama.cpp runtime found".to_string())
        })?;
        let selected_backend = selected.manifest.backend.clone();

        let child_alive = self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_none());
        let same_model = self.loaded_model_path.as_deref() == Some(model_path_string.as_str());
        let same_backend = self
            .active_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.manifest.backend == selected_backend);

        if child_alive && same_model && same_backend {
            // True hot-chat path: reuse the resident model and its KV prompt
            // cache. No runtime probing, process restart or model reload.
            if self.state == ServerState::Ready {
                return Ok(ModelLaunch::Resident(Box::new(self.active_status())));
            }
            return Ok(ModelLaunch::Loading);
        }

        if child_alive {
            let mut loading_fresh = false;
            if self.state == ServerState::LoadingModel {
                let healthy = self
                    .health_target()
                    .as_ref()
                    .is_some_and(|(host, port)| http_health_ok(host, *port));
                if healthy {
                    // Loaded, but its requester has not picked it up yet: reserve it.
                    self.mark_ready();
                } else {
                    loading_fresh = self
                        .load_started
                        .is_some_and(|started| started.elapsed() < MODEL_LOAD_TIMEOUT);
                }
            }
            let claim_pending = self.claim_until.is_some_and(|until| Instant::now() < until);
            if swap_decision(
                loading_fresh,
                self.leases.load(Ordering::SeqCst),
                claim_pending,
            ) == SwapDecision::Wait
            {
                return Ok(ModelLaunch::Busy);
            }
        }

        self.stop()?;
        let server = selected.manifest.binaries.server.as_ref().ok_or_else(|| {
            AppError::RuntimeNotFound("llama-server executable missing".to_string())
        })?;
        let server_path = self.root.resolve_relative(server)?;
        let log_path = self.root.resolve_relative(format!(
            "logs/llama-server-model-{}.log",
            Utc::now().timestamp()
        ))?;
        let stderr_path = self.root.resolve_relative(format!(
            "logs/llama-server-model-{}-stderr.log",
            Utc::now().timestamp()
        ))?;
        let stdout = fs::File::create(log_path)?;
        let stderr = fs::File::create(stderr_path)?;

        self.state = ServerState::LoadingModel;
        let mut args = vec![
            "--host".to_string(),
            config.host.clone(),
            "--port".to_string(),
            config.port.to_string(),
            "--no-webui".to_string(),
            "--model".to_string(),
            model_path_string.clone(),
            "--ctx-size".to_string(),
            config.context_size.to_string(),
            "--threads".to_string(),
            config.threads.to_string(),
            "--threads-batch".to_string(),
            config.threads.to_string(),
            "--batch-size".to_string(),
            config.batch_size.to_string(),
            "--ubatch-size".to_string(),
            config.ubatch_size.to_string(),
            "--parallel".to_string(),
            config.parallelism.to_string(),
        ];
        if let Some(mmproj_path) = discover_mmproj_sibling(&model_path) {
            args.push("--mmproj".to_string());
            args.push(mmproj_path.display().to_string());
        }
        if config.gpu_layers > 0 && selected_backend != BackendKind::Cpu {
            args.push("--gpu-layers".to_string());
            args.push(config.gpu_layers.to_string());
        } else if config.gpu_layers == 0 && config.backend == BackendKind::Cpu {
            // A CPU placement must be explicit: newer llama-server builds otherwise fit
            // layers onto the GPU automatically.
            args.push("--gpu-layers".to_string());
            args.push("0".to_string());
            args.push("--device".to_string());
            args.push("none".to_string());
        }
        if !config.mmap {
            args.push("--no-mmap".to_string());
        }
        if config.mlock {
            args.push("--mlock".to_string());
        }
        // Vulkan flash-attention support varies by llama.cpp build. Avoid
        // paying for a failed launch/retry on the common Windows AMD path.
        if config.flash_attention && selected_backend != BackendKind::Vulkan {
            args.push("--flash-attn".to_string());
        }

        let mut command = Command::new(server_path);
        command
            .args(args)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        hide_console_window(&mut command);
        let child = self
            .spawn_owned(command)
            .map_err(|error| AppError::ModelLoadFailed(error.to_string()))?;

        self.child = Some(child);
        self.endpoint = Some(format!("http://{}:{}", config.host, config.port));
        self.loaded_model_path = Some(model_path_string);
        self.active_runtime = Some(selected);
        self.load_started = Some(Instant::now());
        self.leases = Arc::new(AtomicUsize::new(0));
        self.claim_until = None;
        Ok(ModelLaunch::Loading)
    }

    /// Absolute path of the model the server process was started with, if any.
    pub fn loaded_model_path(&self) -> Option<&str> {
        self.loaded_model_path.as_deref()
    }

    pub fn resolved_model_path(&self, relative: &str) -> Result<String, AppError> {
        Ok(resolve_model_path(&self.root, relative)?
            .display()
            .to_string())
    }

    fn mark_ready(&mut self) {
        self.state = ServerState::Ready;
        self.load_started = None;
        if self.leases.load(Ordering::SeqCst) == 0 {
            self.claim_until = Some(Instant::now() + CLAIM_WINDOW);
        }
    }

    /// Takes a use lease on the resident model and ends its claim window.
    pub fn lease(&mut self) -> ModelLease {
        self.claim_until = None;
        self.leases.fetch_add(1, Ordering::SeqCst);
        ModelLease(self.leases.clone())
    }

    pub fn endpoint(&self) -> Option<String> {
        self.endpoint.clone()
    }

    pub fn has_process(&self) -> bool {
        self.child.is_some()
    }

    /// True while the server process is still running (it may have crashed or been killed).
    pub fn is_process_alive(&mut self) -> bool {
        self.child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_none())
    }

    pub fn state(&self) -> ServerState {
        self.state.clone()
    }

    fn health_target(&self) -> Option<(String, u16)> {
        let endpoint = self.endpoint.as_deref()?;
        let address = endpoint.strip_prefix("http://")?;
        let (host, port) = address.rsplit_once(':')?;
        Some((host.to_string(), port.parse().ok()?))
    }

    /// Non-blocking readiness check for `expected_model_path` (an absolute path as returned
    /// by [`Self::resolved_model_path`]). Marks the server ready once `/health` answers.
    pub fn probe_model_server(&mut self, expected_model_path: &str) -> ModelProbe {
        if self.loaded_model_path.as_deref() != Some(expected_model_path) {
            return ModelProbe::Failed(
                "the runtime switched to a different model while this one was loading".to_string(),
            );
        }
        let exited = match self.child.as_mut() {
            None => true,
            Some(child) => child.try_wait().ok().flatten().is_some(),
        };
        if exited {
            self.state = ServerState::Failed;
            return ModelProbe::Failed(
                "llama-server exited while loading the model; see the llama-server logs"
                    .to_string(),
            );
        }
        let Some((host, port)) = self.health_target() else {
            return ModelProbe::Failed("llama-server endpoint missing".to_string());
        };
        if self.state == ServerState::Ready || http_health_ok(&host, port) {
            if self.state != ServerState::Ready {
                self.mark_ready();
            }
            return ModelProbe::Ready(self.endpoint.clone().unwrap_or_default());
        }
        ModelProbe::Loading
    }

    pub fn stop(&mut self) -> Result<(), AppError> {
        self.state = ServerState::Stopping;
        self.load_started = None;
        // Leases on the stopped model no longer protect anything.
        self.leases = Arc::new(AtomicUsize::new(0));
        self.claim_until = None;
        if let Some(mut child) = self.child.take() {
            let pid = child.id();
            // An already exited child reports an error here; it still must be reaped.
            let killed = child.kill();
            let _ = child.wait();
            process_ownership::forget(&Self::ownership_registry(&self.root), pid);
            if let Err(error) = killed {
                if child_still_running(pid) {
                    return Err(AppError::RuntimeStartFailed(error.to_string()));
                }
            }
        }
        self.endpoint = None;
        self.loaded_model_path = None;
        self.active_runtime = None;
        self.state = ServerState::Stopped;
        Ok(())
    }
}

fn child_still_running(pid: u32) -> bool {
    process_ownership::identity(pid).is_some()
}

/// Loads `config` into the runtime behind `runtime` and waits until it is healthy, holding
/// the lock only for short launch/probe steps so status queries and the other runtime are
/// never blocked by a slow load. If another model is mid-load, waits for that load to finish
/// before switching, so concurrent callers never kill each other's loads. Blocking: call
/// from synchronous code or `spawn_blocking`.
pub fn ensure_model_ready(
    runtime: &Mutex<LlamaRuntimeManager>,
    hardware: &HardwareProfile,
    config: &ModelLaunchConfig,
) -> Result<(LlamaRuntimeStatus, ModelLease), AppError> {
    let lock = || {
        runtime
            .lock()
            .map_err(|_| AppError::internal("runtime lock poisoned"))
    };
    let expected = lock()?.resolved_model_path(&config.model_path)?;
    let started = Instant::now();
    let mut launched = false;
    loop {
        {
            let mut manager = lock()?;
            if !launched {
                match manager.launch_model_server(hardware, config)? {
                    ModelLaunch::Resident(status) => return Ok((*status, manager.lease())),
                    ModelLaunch::Loading => launched = true,
                    ModelLaunch::Busy => {}
                }
            }
            if launched {
                match manager.probe_model_server(&expected) {
                    ModelProbe::Ready(_) => {
                        let lease = manager.lease();
                        return Ok((manager.active_status(), lease));
                    }
                    ModelProbe::Failed(reason) => return Err(AppError::ModelLoadFailed(reason)),
                    ModelProbe::Loading => {}
                }
            }
        }
        if started.elapsed() >= MODEL_LOAD_TIMEOUT {
            if launched {
                let mut manager = lock()?;
                if manager.loaded_model_path() == Some(expected.as_str()) {
                    let _ = manager.stop();
                }
            }
            return Err(AppError::ModelLoadFailed(format!(
                "the model did not finish loading within {} seconds",
                MODEL_LOAD_TIMEOUT.as_secs()
            )));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

impl Drop for LlamaRuntimeManager {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub struct RuntimeRegistry<'a> {
    root: &'a PortableRootManager,
}

impl<'a> RuntimeRegistry<'a> {
    pub fn new(root: &'a PortableRootManager) -> Self {
        Self { root }
    }

    pub fn discover(&self) -> Result<Vec<RuntimeManifest>, AppError> {
        let manifest_dir = self.root.resolve_relative("runtimes/llama/manifests")?;
        if !manifest_dir.exists() {
            return Ok(Vec::new());
        }

        let mut manifests = Vec::new();
        for entry in fs::read_dir(manifest_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let content = fs::read_to_string(path)?;
            let manifest =
                serde_json::from_str::<RuntimeManifest>(content.trim_start_matches('\u{feff}'))
                    .map_err(|error| AppError::RuntimeStartFailed(error.to_string()))?;
            manifests.push(manifest);
        }
        manifests.sort_by(|left, right| left.runtime_name.cmp(&right.runtime_name));
        Ok(manifests)
    }
}

impl RuntimeSelector {
    pub fn select(
        runtimes: &[RuntimeValidation],
        hardware: &HardwareProfile,
    ) -> Option<RuntimeValidation> {
        let priority = preferred_backend_order(hardware);
        priority.into_iter().find_map(|backend| {
            runtimes
                .iter()
                .find(|runtime| runtime.usable && runtime.manifest.backend == backend)
                .cloned()
        })
    }
}

pub(crate) fn preferred_backend_order(hardware: &HardwareProfile) -> Vec<BackendKind> {
    let mut order = Vec::new();
    let has_nvidia = hardware
        .gpus
        .iter()
        .any(|gpu| gpu.vendor == GpuVendor::Nvidia && !gpu.is_software);
    let has_amd = hardware
        .gpus
        .iter()
        .any(|gpu| gpu.vendor == GpuVendor::Amd && !gpu.is_software);
    let has_intel = hardware
        .gpus
        .iter()
        .any(|gpu| gpu.vendor == GpuVendor::Intel && !gpu.is_software);

    if has_nvidia {
        order.push(BackendKind::Cuda);
    }
    if has_amd {
        // Windows AMD packages are Vulkan-first in OpenMindAI. This keeps the
        // runtime choice aligned with the launch planner on RX 580-class GPUs.
        if cfg!(target_os = "windows") {
            order.push(BackendKind::Vulkan);
            order.push(BackendKind::Hip);
        } else {
            order.push(BackendKind::Hip);
        }
    }
    if has_intel {
        order.push(BackendKind::Sycl);
    }
    if !order.contains(&BackendKind::Vulkan) {
        order.push(BackendKind::Vulkan);
    }
    order.push(BackendKind::Cpu);
    order
}

/// Fast metadata validation. Model launch and `/health` are the source of
/// truth for executable compatibility. Spawning `--version`/`--list-devices`
/// on every inventory read made both startup and first-token latency depend on
/// subprocess timeouts.
pub fn validate_manifest(
    root: &PortableRootManager,
    manifest: RuntimeManifest,
) -> Result<RuntimeValidation, AppError> {
    let server_exists = file_exists_under_root(root, manifest.binaries.server.as_deref())?;
    let cli_exists = file_exists_under_root(root, manifest.binaries.cli.as_deref())?;
    let bench_exists = file_exists_under_root(root, manifest.binaries.bench.as_deref())?;
    let usable = server_exists
        && !matches!(
            manifest.status,
            RuntimeStatus::Disabled | RuntimeStatus::Invalid | RuntimeStatus::Missing
        );
    let version_output = usable.then(|| manifest.version.clone());
    let message = if usable {
        "runtime ready for launch".to_string()
    } else if !server_exists {
        "required server executable missing".to_string()
    } else {
        "runtime disabled or invalid".to_string()
    };

    Ok(RuntimeValidation {
        manifest,
        server_exists,
        cli_exists,
        bench_exists,
        version_output,
        device_output: None,
        usable,
        message,
    })
}

fn file_exists_under_root(
    root: &PortableRootManager,
    relative: Option<&str>,
) -> Result<bool, AppError> {
    let Some(relative) = relative else {
        return Ok(false);
    };
    Ok(root.resolve_relative(relative)?.is_file())
}

fn resolve_model_path(root: &PortableRootManager, path: &str) -> Result<PathBuf, AppError> {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        Ok(candidate.to_path_buf())
    } else {
        root.resolve_relative(path)
    }
}

fn discover_mmproj_sibling(model_path: &Path) -> Option<PathBuf> {
    let parent = model_path.parent()?;
    let mut candidates = fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.to_ascii_lowercase().starts_with("mmproj-"))
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.into_iter().next()
}

fn hide_console_window(_command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        _command.creation_flags(CREATE_NO_WINDOW);
    }
}

pub fn allocate_local_port() -> Result<u16, AppError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| AppError::RuntimeStartFailed(error.to_string()))?;
    let addr = listener
        .local_addr()
        .map_err(|error| AppError::RuntimeStartFailed(error.to_string()))?;
    Ok(addr.port())
}

fn wait_for_localhost(port: u16, timeout: Duration) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let start = Instant::now();
    while start.elapsed() < timeout {
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn http_health_ok(host: &str, port: u16) -> bool {
    let Ok(mut addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    let Some(addr) = addrs.next() else {
        return false;
    };
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(350))
    else {
        return false;
    };
    if stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .is_err()
    {
        return false;
    }
    let request = format!("GET /health HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{CpuProfile, MemoryProfile};

    #[test]
    fn a_different_model_waits_for_load_use_and_claim() {
        // Still loading: never killed halfway.
        assert_eq!(swap_decision(true, 0, false), SwapDecision::Wait);
        // Loaded and in use by a chat or agent request: not swapped out mid-request.
        assert_eq!(swap_decision(false, 1, false), SwapDecision::Wait);
        // Loaded but its requester has not picked it up yet: reserved for it.
        assert_eq!(swap_decision(false, 0, true), SwapDecision::Wait);
        // Idle: safe to switch.
        assert_eq!(swap_decision(false, 0, false), SwapDecision::Replace);
    }

    #[test]
    fn leases_count_users_and_end_the_claim_window() {
        let root =
            PortableRootManager::from_root(std::env::temp_dir().join("openmindai-lease-test"));
        let mut manager = LlamaRuntimeManager::new(root);
        manager.mark_ready();
        assert!(manager.claim_until.is_some());
        let first = manager.lease();
        let second = manager.lease();
        assert!(manager.claim_until.is_none());
        assert_eq!(manager.leases.load(Ordering::SeqCst), 2);
        drop(first);
        assert_eq!(manager.leases.load(Ordering::SeqCst), 1);
        // A model that becomes ready while still leased gets no claim window.
        manager.mark_ready();
        assert!(manager.claim_until.is_none());
        drop(second);
        assert_eq!(manager.leases.load(Ordering::SeqCst), 0);
        // Stopping forgets leases of the old model.
        let stale = manager.lease();
        manager.stop().unwrap();
        assert_eq!(manager.leases.load(Ordering::SeqCst), 0);
        drop(stale);
        assert_eq!(manager.leases.load(Ordering::SeqCst), 0);
    }

    fn manifest(backend: BackendKind, server: Option<&str>) -> RuntimeManifest {
        RuntimeManifest {
            runtime_name: format!("llama-{backend:?}"),
            version: "test".to_string(),
            platform: "windows".to_string(),
            architecture: "x86_64".to_string(),
            backend,
            source: "test".to_string(),
            installed_at: "test".to_string(),
            binaries: RuntimeBinaries {
                server: server.map(str::to_string),
                cli: None,
                bench: None,
            },
            checksum: None,
            status: RuntimeStatus::Available,
        }
    }

    fn hardware(vendor: GpuVendor) -> HardwareProfile {
        HardwareProfile {
            operating_system: "test".to_string(),
            architecture: "x86_64".to_string(),
            cpu: CpuProfile {
                name: "cpu".to_string(),
                physical_cores: Some(4),
                logical_threads: 8,
            },
            memory: MemoryProfile {
                total_bytes: 16,
                available_bytes: 8,
            },
            gpus: vec![crate::hardware::GpuInfo {
                id: "gpu0".to_string(),
                name: "gpu".to_string(),
                vendor,
                vendor_id: None,
                device_id: None,
                subsystem_id: None,
                revision: None,
                dedicated_vram_bytes: Some(8),
                dedicated_system_memory_bytes: Some(0),
                shared_memory_bytes: Some(4),
                luid: None,
                is_discrete: true,
                is_integrated: false,
                is_software: false,
                available_backends: vec![BackendKind::Cpu],
                recommended_backend: BackendKind::Cpu,
            }],
            primary_gpu: Some("gpu0".to_string()),
            recommended_inference_gpu: Some("gpu0".to_string()),
            backends: crate::hardware::BackendProfile {
                cpu: true,
                cuda: false,
                vulkan: true,
                sycl: false,
                hip: false,
                metal: false,
            },
            detection_complete: true,
        }
    }

    #[test]
    fn selector_prefers_cuda_when_valid_on_nvidia() {
        let runtimes = vec![
            RuntimeValidation {
                manifest: manifest(BackendKind::Vulkan, Some("vulkan.exe")),
                server_exists: true,
                cli_exists: true,
                bench_exists: false,
                version_output: Some("version".to_string()),
                device_output: None,
                usable: true,
                message: "ok".to_string(),
            },
            RuntimeValidation {
                manifest: manifest(BackendKind::Cuda, Some("cuda.exe")),
                server_exists: true,
                cli_exists: true,
                bench_exists: false,
                version_output: Some("version".to_string()),
                device_output: None,
                usable: true,
                message: "ok".to_string(),
            },
        ];

        let selected = RuntimeSelector::select(&runtimes, &hardware(GpuVendor::Nvidia)).unwrap();
        assert_eq!(selected.manifest.backend, BackendKind::Cuda);
    }

    #[test]
    fn selector_falls_back_to_vulkan_then_cpu() {
        let runtimes = vec![
            RuntimeValidation {
                manifest: manifest(BackendKind::Cuda, Some("cuda.exe")),
                server_exists: true,
                cli_exists: true,
                bench_exists: false,
                version_output: None,
                device_output: None,
                usable: false,
                message: "bad".to_string(),
            },
            RuntimeValidation {
                manifest: manifest(BackendKind::Vulkan, Some("vulkan.exe")),
                server_exists: true,
                cli_exists: true,
                bench_exists: false,
                version_output: Some("version".to_string()),
                device_output: None,
                usable: true,
                message: "ok".to_string(),
            },
        ];

        let selected = RuntimeSelector::select(&runtimes, &hardware(GpuVendor::Nvidia)).unwrap();
        assert_eq!(selected.manifest.backend, BackendKind::Vulkan);
    }

    #[test]
    fn missing_manifest_binary_is_invalid() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let validation = validate_manifest(
            &root,
            manifest(BackendKind::Cpu, Some("runtimes/llama/missing.exe")),
        )
        .unwrap();
        assert!(!validation.usable);
        assert!(!validation.server_exists);
    }

    #[test]
    fn runtime_paths_remain_under_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        assert!(file_exists_under_root(&root, Some("../bad.exe")).is_err());
    }

    #[test]
    fn allocates_localhost_port() {
        let port = allocate_local_port().unwrap();
        assert!(port > 0);
    }

    #[test]
    fn real_staged_runtime_validates_when_present() {
        let _guard = crate::portable_root::tests::ENV_LOCK.lock().unwrap();
        let Ok(root) = PortableRootManager::resolve() else {
            return;
        };
        root.ensure_directories().unwrap();
        let hardware = crate::hardware::HardwareProfiler::detect();
        let mut manager = LlamaRuntimeManager::new(root);
        let inventory = manager.inventory(&hardware).unwrap();
        if inventory.runtimes.is_empty() {
            return;
        }

        assert!(inventory.runtimes.iter().any(|runtime| runtime.usable));
        let status = manager.start_server(&hardware).unwrap();
        assert!(matches!(
            status.state,
            ServerState::Ready | ServerState::Running
        ));
        manager.stop().unwrap();
    }
}
