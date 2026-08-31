use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, ToSocketAddrs},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
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
};

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

pub struct LlamaRuntimeManager {
    root: PortableRootManager,
    child: Option<Child>,
    endpoint: Option<String>,
    state: ServerState,
    loaded_model_path: Option<String>,
    active_runtime: Option<RuntimeValidation>,
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
        }
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
            AppError::RuntimeNotFound("no validated llama.cpp runtime found".to_string())
        })?;
        let server = selected
            .manifest
            .binaries
            .server
            .as_ref()
            .ok_or_else(|| AppError::RuntimeNotFound("llama-server.exe missing".to_string()))?;
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
        let child = command
            .spawn()
            .map_err(|error| AppError::RuntimeStartFailed(error.to_string()))?;

        self.child = Some(child);
        self.endpoint = Some(endpoint.clone());

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
            // llama-server without a model may stay alive but not serve health. The process
            // lifecycle is still owned and can be stopped cleanly in this milestone.
            self.state = ServerState::Running;
        }

        self.status(hardware)
    }

    pub fn ensure_model_server(
        &mut self,
        hardware: &HardwareProfile,
        config: &ModelLaunchConfig,
    ) -> Result<LlamaRuntimeStatus, AppError> {
        let model_path = resolve_model_path(&self.root, &config.model_path)?;
        let model_path_string = model_path.display().to_string();

        if self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_none())
            && self.loaded_model_path.as_deref() == Some(model_path_string.as_str())
        {
            // Hot chat path: the same model is already loaded and the child is
            // alive. Re-running status() here used to rediscover runtimes and
            // spawn --version / --list-devices probes before every message.
            return Ok(self.active_status());
        }

        self.stop()?;
        let selected = self.inventory(hardware)?.selected.ok_or_else(|| {
            AppError::RuntimeNotFound("no validated llama.cpp runtime found".to_string())
        })?;
        let server = selected
            .manifest
            .binaries
            .server
            .as_ref()
            .ok_or_else(|| AppError::RuntimeNotFound("llama-server.exe missing".to_string()))?;
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
        if config.gpu_layers > 0 {
            args.push("--gpu-layers".to_string());
            args.push(config.gpu_layers.to_string());
        }
        if !config.mmap {
            args.push("--no-mmap".to_string());
        }
        if config.mlock {
            args.push("--mlock".to_string());
        }
        if config.flash_attention {
            args.push("--flash-attn".to_string());
        }

        let mut command = Command::new(server_path);
        command
            .args(args)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        hide_console_window(&mut command);
        let child = command
            .spawn()
            .map_err(|error| AppError::ModelLoadFailed(error.to_string()))?;

        self.child = Some(child);
        self.endpoint = Some(format!("http://{}:{}", config.host, config.port));
        self.loaded_model_path = Some(model_path_string);
        self.active_runtime = Some(selected);

        if wait_for_model_health(&config.host, config.port, Duration::from_secs(120)) {
            self.state = ServerState::Ready;
            Ok(self.active_status())
        } else {
            self.state = ServerState::Failed;
            Err(AppError::ModelLoadFailed(
                "llama-server did not become healthy after model launch".to_string(),
            ))
        }
    }

    pub fn stop(&mut self) -> Result<(), AppError> {
        self.state = ServerState::Stopping;
        if let Some(mut child) = self.child.take() {
            child
                .kill()
                .map_err(|error| AppError::RuntimeStartFailed(error.to_string()))?;
            let _ = child.wait();
        }
        self.endpoint = None;
        self.loaded_model_path = None;
        self.active_runtime = None;
        self.state = ServerState::Stopped;
        Ok(())
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
        order.push(BackendKind::Hip);
    }
    if has_intel {
        order.push(BackendKind::Sycl);
    }
    order.push(BackendKind::Vulkan);
    order.push(BackendKind::Cpu);
    order
}

pub fn validate_manifest(
    root: &PortableRootManager,
    manifest: RuntimeManifest,
) -> Result<RuntimeValidation, AppError> {
    let server_exists = file_exists_under_root(root, manifest.binaries.server.as_deref())?;
    let cli_exists = file_exists_under_root(root, manifest.binaries.cli.as_deref())?;
    let bench_exists = file_exists_under_root(root, manifest.binaries.bench.as_deref())?;
    let probe_binary = manifest
        .binaries
        .cli
        .as_deref()
        .or(manifest.binaries.server.as_deref());
    let version_output = probe_binary.and_then(|relative| {
        run_probe(root, relative, &["--version"], Duration::from_secs(8)).ok()
    });
    let device_output = probe_binary.and_then(|relative| {
        run_probe(root, relative, &["--list-devices"], Duration::from_secs(12)).ok()
    });
    let usable = (server_exists || cli_exists)
        && manifest.status != RuntimeStatus::Disabled
        && version_output.is_some();
    let message = if usable {
        "runtime validated".to_string()
    } else if !server_exists && !cli_exists {
        "required executable missing".to_string()
    } else {
        "runtime probe failed".to_string()
    };

    Ok(RuntimeValidation {
        manifest,
        server_exists,
        cli_exists,
        bench_exists,
        version_output,
        device_output,
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

fn run_probe(
    root: &PortableRootManager,
    relative: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<String, AppError> {
    let path = root.resolve_relative(relative)?;
    let mut command = Command::new(path);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| AppError::RuntimeStartFailed(error.to_string()))?;

    let start = Instant::now();
    while start.elapsed() < timeout {
        if child
            .try_wait()
            .map_err(|error| AppError::RuntimeStartFailed(error.to_string()))?
            .is_some()
        {
            let output = child
                .wait_with_output()
                .map_err(|error| AppError::RuntimeStartFailed(error.to_string()))?;
            let mut text = String::from_utf8_lossy(&output.stdout).to_string();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            return Ok(text.trim().to_string());
        }
        thread::sleep(Duration::from_millis(50));
    }

    let _ = child.kill();
    Err(AppError::RuntimeStartFailed(
        "runtime probe timed out".to_string(),
    ))
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

/// Polls llama-server's `/health` endpoint rather than just the TCP port.
/// The HTTP listener accepts connections the moment the process starts --
/// well before the model finishes loading into memory -- so a bare
/// `wait_for_localhost` marks the server "Ready" while `/v1/chat/completions`
/// is still answering 503, which is what surfaced as a raw 503 to users.
/// `/health` only returns 200 once the model is actually loaded.
fn wait_for_model_health(host: &str, port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if http_health_ok(host, port) {
            return true;
        }
        thread::sleep(Duration::from_millis(300));
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
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500))
    else {
        return false;
    };
    if stream
        .set_read_timeout(Some(Duration::from_secs(2)))
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
        // Resolving the real OPENMINDAI_ROOT races with portable_root's own
        // tests, which mutate that same process-global env var — share their
        // lock so this test observes a stable environment.
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
