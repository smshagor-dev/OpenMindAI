use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};

#[cfg(target_os = "windows")]
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
#[cfg(target_os = "windows")]
use serde::Deserialize;
use serde::Serialize;
use tokio::process::Command;
#[cfg(target_os = "windows")]
use tokio::time::sleep;
#[cfg(target_os = "windows")]
use uuid::Uuid;

use crate::app_error::AppError;

const RUNTIME_CWD_MARKER: &str = "__OPENMIND_RUNTIME_CWD__";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const SAFE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxCapability {
    pub platform: String,
    pub provider: Option<String>,
    pub available: bool,
    pub strong_isolation: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ShellExecutionResult {
    pub cwd: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
    pub timed_out: bool,
    pub truncated: bool,
    pub backend: String,
    pub isolated: bool,
    pub network_disabled: bool,
}

#[derive(Debug, Clone, Copy)]
enum IsolationProvider {
    Bubblewrap,
    MacSandbox,
    WindowsSandbox,
}

impl IsolationProvider {
    fn label(self) -> &'static str {
        match self {
            Self::Bubblewrap => "bubblewrap",
            Self::MacSandbox => "sandbox-exec",
            Self::WindowsSandbox => "Windows Sandbox",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Bubblewrap => {
                "Strong local isolation is ready: bubblewrap maps only the attached workspace writable, hides the user home, clears inherited environment variables, and disables networking."
            }
            Self::MacSandbox => {
                "Strong local isolation is ready: sandbox-exec restricts writes to the attached workspace and temporary files, blocks user-home reads outside the workspace, clears inherited environment variables, and disables networking."
            }
            Self::WindowsSandbox => {
                "Strong microVM isolation is ready: Windows Sandbox maps only the attached workspace plus an ephemeral control folder, disables networking and device redirection, and runs commands inside the disposable VM."
            }
        }
    }
}

static DETECTED_PROVIDER: OnceLock<Option<IsolationProvider>> = OnceLock::new();

pub fn sandbox_capability() -> SandboxCapability {
    let provider = detected_provider();
    SandboxCapability {
        platform: env::consts::OS.to_string(),
        provider: provider.map(|value| value.label().to_string()),
        available: provider.is_some(),
        strong_isolation: provider.is_some(),
        message: provider
            .map(|value| value.message().to_string())
            .unwrap_or_else(unavailable_message),
    }
}

pub async fn run_isolated_shell(
    workspace_root: &Path,
    cwd: &Path,
    command: &str,
    timeout_secs: u64,
    max_output_chars: usize,
) -> Result<ShellExecutionResult, AppError> {
    let (workspace_root, cwd) = validate_isolated_scope(workspace_root, cwd)?;
    let provider = detected_provider().ok_or_else(|| {
        AppError::internal(format!(
            "isolated terminal is unavailable on this machine: {} No host fallback was attempted.",
            unavailable_message()
        ))
    })?;

    match provider {
        IsolationProvider::Bubblewrap => {
            run_bubblewrap(
                &workspace_root,
                &cwd,
                command,
                timeout_secs,
                max_output_chars,
            )
            .await
        }
        IsolationProvider::MacSandbox => {
            run_macos_sandbox(
                &workspace_root,
                &cwd,
                command,
                timeout_secs,
                max_output_chars,
            )
            .await
        }
        IsolationProvider::WindowsSandbox => {
            run_windows_sandbox(
                &workspace_root,
                &cwd,
                command,
                timeout_secs,
                max_output_chars,
            )
            .await
        }
    }
}

pub async fn run_host_shell(
    cwd: &Path,
    command: &str,
    timeout_secs: u64,
    max_output_chars: usize,
) -> Result<ShellExecutionResult, AppError> {
    let cwd = fs::canonicalize(cwd)?;
    if !cwd.is_dir() {
        return Err(AppError::internal(
            "terminal working directory is not a directory",
        ));
    }
    let command = command.trim();
    if command.is_empty() {
        return Err(AppError::internal("terminal command cannot be empty"));
    }

    let started = Instant::now();
    let mut process = host_shell_process(command, &cwd);
    process.kill_on_drop(true);
    let output =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), process.output()).await {
            Ok(result) => result?,
            Err(_) => {
                return Ok(ShellExecutionResult {
                    cwd: display_path(&cwd),
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: format!("Command timed out after {timeout_secs} seconds."),
                    duration_ms: started.elapsed().as_millis(),
                    timed_out: true,
                    truncated: false,
                    backend: "host-explicit".to_string(),
                    isolated: false,
                    network_disabled: false,
                });
            }
        };

    let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let resolved_cwd = take_cwd_marker(&mut stdout).unwrap_or_else(|| display_path(&cwd));
    let (stdout, stdout_truncated) = truncate_chars(&stdout, max_output_chars);
    let (stderr, stderr_truncated) = truncate_chars(&stderr, max_output_chars);
    Ok(ShellExecutionResult {
        cwd: resolved_cwd,
        exit_code: output.status.code().unwrap_or(-1),
        stdout,
        stderr,
        duration_ms: started.elapsed().as_millis(),
        timed_out: false,
        truncated: stdout_truncated || stderr_truncated,
        backend: "host-explicit".to_string(),
        isolated: false,
        network_disabled: false,
    })
}

fn detected_provider() -> Option<IsolationProvider> {
    *DETECTED_PROVIDER.get_or_init(|| match env::consts::OS {
        "linux" if bubblewrap_path().is_some() => Some(IsolationProvider::Bubblewrap),
        "macos" if macos_sandbox_path().is_some() => Some(IsolationProvider::MacSandbox),
        "windows" if windows_sandbox_path().is_some() => Some(IsolationProvider::WindowsSandbox),
        _ => None,
    })
}

fn unavailable_message() -> String {
    match env::consts::OS {
        "linux" => {
            "Install bubblewrap (`bwrap`) in /usr/bin, /bin, or /usr/local/bin to enable strong workspace isolation.".to_string()
        }
        "macos" => {
            "This macOS installation does not provide /usr/bin/sandbox-exec, so strong local shell isolation is unavailable.".to_string()
        }
        "windows" => {
            "Enable the Windows Sandbox optional feature to provide the disposable microVM backend.".to_string()
        }
        platform => format!("No strong isolation backend is implemented for {platform}."),
    }
}

fn validate_isolated_scope(
    workspace_root: &Path,
    cwd: &Path,
) -> Result<(PathBuf, PathBuf), AppError> {
    let root = fs::canonicalize(workspace_root)?;
    if !root.is_dir() {
        return Err(AppError::internal(
            "isolated execution workspace root is not a directory",
        ));
    }
    let cwd = fs::canonicalize(cwd)?;
    if !cwd.is_dir() || !cwd.starts_with(&root) {
        return Err(AppError::internal(
            "isolated terminal working directory must remain inside the attached workspace",
        ));
    }
    Ok((root, cwd))
}

fn bubblewrap_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        trusted_executable(&["/usr/bin/bwrap", "/bin/bwrap", "/usr/local/bin/bwrap"])
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn macos_sandbox_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        trusted_executable(&["/usr/bin/sandbox-exec"])
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn windows_sandbox_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let root = env::var_os("SystemRoot").map(PathBuf::from)?;
        let path = root.join("System32/WindowsSandbox.exe");
        path.is_file().then_some(path)
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn trusted_executable(candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
        .and_then(|candidate| fs::canonicalize(candidate).ok())
}

#[cfg(target_os = "linux")]
async fn run_bubblewrap(
    workspace_root: &Path,
    cwd: &Path,
    command: &str,
    timeout_secs: u64,
    max_output_chars: usize,
) -> Result<ShellExecutionResult, AppError> {
    let executable = bubblewrap_path()
        .ok_or_else(|| AppError::internal("bubblewrap disappeared after capability detection"))?;
    let relative = cwd
        .strip_prefix(workspace_root)
        .map_err(|_| AppError::internal("isolated cwd escaped workspace"))?;
    let sandbox_cwd = Path::new("/workspace").join(relative);
    let wrapped = shell_wrapper(command);

    let mut process = Command::new(executable);
    process
        .arg("--die-with-parent")
        .arg("--new-session")
        .arg("--unshare-all")
        .arg("--clearenv")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--tmpfs")
        .arg("/tmp")
        .arg("--tmpfs")
        .arg("/run")
        .arg("--dir")
        .arg("/workspace");

    for path in ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/opt"] {
        if Path::new(path).exists() {
            process.arg("--ro-bind").arg(path).arg(path);
        }
    }

    process
        .arg("--bind")
        .arg(workspace_root)
        .arg("/workspace")
        .arg("--chdir")
        .arg(sandbox_cwd)
        .arg("--setenv")
        .arg("HOME")
        .arg("/tmp")
        .arg("--setenv")
        .arg("TMPDIR")
        .arg("/tmp")
        .arg("--setenv")
        .arg("PATH")
        .arg(SAFE_PATH)
        .arg("--setenv")
        .arg("LANG")
        .arg("C.UTF-8")
        .arg("--setenv")
        .arg("OPENMINDAI_ISOLATED")
        .arg("1")
        .arg("--")
        .arg("/bin/sh")
        .arg("-lc")
        .arg(wrapped)
        .kill_on_drop(true);

    let started = Instant::now();
    let output =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), process.output()).await {
            Ok(result) => result?,
            Err(_) => {
                return Ok(timeout_result(
                    workspace_root,
                    cwd,
                    timeout_secs,
                    started,
                    "bubblewrap",
                ));
            }
        };
    finish_isolated_output(IsolatedOutput {
        workspace_root,
        cwd,
        exit_code: output.status.code().unwrap_or(-1),
        stdout: &output.stdout,
        stderr: &output.stderr,
        started,
        max_output_chars,
        backend: "bubblewrap",
        sandbox_workspace_prefix: Some("/workspace"),
    })
}

#[cfg(not(target_os = "linux"))]
async fn run_bubblewrap(
    _workspace_root: &Path,
    _cwd: &Path,
    _command: &str,
    _timeout_secs: u64,
    _max_output_chars: usize,
) -> Result<ShellExecutionResult, AppError> {
    Err(AppError::internal(
        "bubblewrap backend is only available on Linux",
    ))
}

#[cfg(target_os = "macos")]
async fn run_macos_sandbox(
    workspace_root: &Path,
    cwd: &Path,
    command: &str,
    timeout_secs: u64,
    max_output_chars: usize,
) -> Result<ShellExecutionResult, AppError> {
    let executable = macos_sandbox_path()
        .ok_or_else(|| AppError::internal("sandbox-exec disappeared after capability detection"))?;
    let workspace = sandbox_profile_escape(&display_path(workspace_root));
    let profile = format!(
        "(version 1)\n\
         (deny default)\n\
         (allow process*)\n\
         (allow signal)\n\
         (allow sysctl-read)\n\
         (allow mach-lookup)\n\
         (allow ipc-posix*)\n\
         (allow file-read-metadata)\n\
         (allow file-read*\n\
           (subpath \"/System\")\n\
           (subpath \"/usr\")\n\
           (subpath \"/bin\")\n\
           (subpath \"/sbin\")\n\
           (subpath \"/Library\")\n\
           (subpath \"/Applications/Xcode.app\")\n\
           (subpath \"/private/etc\")\n\
           (subpath \"/private/var/db/dyld\")\n\
           (subpath \"/dev\")\n\
           (subpath \"{workspace}\"))\n\
         (allow file-write*\n\
           (subpath \"{workspace}\")\n\
           (subpath \"/private/tmp\")\n\
           (subpath \"/tmp\"))\n\
         (deny network*)"
    );
    let wrapped = shell_wrapper(command);
    let mut process = Command::new(executable);
    process
        .arg("-p")
        .arg(profile)
        .arg("/bin/sh")
        .arg("-lc")
        .arg(wrapped)
        .current_dir(cwd)
        .env_clear()
        .env("HOME", "/private/tmp")
        .env("TMPDIR", "/private/tmp")
        .env("PATH", SAFE_PATH)
        .env("LANG", "C.UTF-8")
        .env("OPENMINDAI_ISOLATED", "1")
        .kill_on_drop(true);

    let started = Instant::now();
    let output =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), process.output()).await {
            Ok(result) => result?,
            Err(_) => {
                return Ok(timeout_result(
                    workspace_root,
                    cwd,
                    timeout_secs,
                    started,
                    "sandbox-exec",
                ));
            }
        };
    finish_isolated_output(IsolatedOutput {
        workspace_root,
        cwd,
        exit_code: output.status.code().unwrap_or(-1),
        stdout: &output.stdout,
        stderr: &output.stderr,
        started,
        max_output_chars,
        backend: "sandbox-exec",
        sandbox_workspace_prefix: None,
    })
}

#[cfg(not(target_os = "macos"))]
async fn run_macos_sandbox(
    _workspace_root: &Path,
    _cwd: &Path,
    _command: &str,
    _timeout_secs: u64,
    _max_output_chars: usize,
) -> Result<ShellExecutionResult, AppError> {
    Err(AppError::internal(
        "sandbox-exec backend is only available on macOS",
    ))
}

#[cfg(target_os = "windows")]
async fn run_windows_sandbox(
    workspace_root: &Path,
    cwd: &Path,
    command: &str,
    timeout_secs: u64,
    max_output_chars: usize,
) -> Result<ShellExecutionResult, AppError> {
    let executable = windows_sandbox_path().ok_or_else(|| {
        AppError::internal("Windows Sandbox disappeared after capability detection")
    })?;
    let relative = cwd
        .strip_prefix(workspace_root)
        .map_err(|_| AppError::internal("isolated cwd escaped workspace"))?;
    let sandbox_cwd = if relative.as_os_str().is_empty() {
        "C:\\OpenMindWorkspace".to_string()
    } else {
        format!(
            "C:\\OpenMindWorkspace\\{}",
            relative.to_string_lossy().replace('/', "\\")
        )
    };

    let control_dir = env::temp_dir().join(format!("openmindai-sandbox-{}", Uuid::new_v4()));
    fs::create_dir_all(&control_dir)?;
    let stdout_path = control_dir.join("stdout.txt");
    let stderr_path = control_dir.join("stderr.txt");
    let result_path = control_dir.join("result.json");
    let launcher_path = control_dir.join("launcher.ps1");
    let config_path = control_dir.join("run.wsb");

    let encoded_command = powershell_encoded_command(command);
    let launcher = format!(
        "$ErrorActionPreference = 'Stop'\r\n\
         $stdout = 'C:\\OpenMindControl\\stdout.txt'\r\n\
         $stderr = 'C:\\OpenMindControl\\stderr.txt'\r\n\
         $result = 'C:\\OpenMindControl\\result.json'\r\n\
         try {{\r\n\
           $process = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand','{encoded_command}') -WorkingDirectory '{}' -RedirectStandardOutput $stdout -RedirectStandardError $stderr -Wait -PassThru\r\n\
           @{{ exitCode = [int]$process.ExitCode }} | ConvertTo-Json -Compress | Set-Content -LiteralPath $result -Encoding UTF8\r\n\
         }} catch {{\r\n\
           $_ | Out-String | Set-Content -LiteralPath $stderr -Encoding UTF8\r\n\
           @{{ exitCode = 1 }} | ConvertTo-Json -Compress | Set-Content -LiteralPath $result -Encoding UTF8\r\n\
         }}\r\n\
         shutdown.exe /s /t 0 /f\r\n",
        powershell_single_quote(&sandbox_cwd)
    );
    fs::write(&launcher_path, launcher.as_bytes())?;

    let config = format!(
        "<Configuration>\r\n\
           <VGpu>Disable</VGpu>\r\n\
           <Networking>Disable</Networking>\r\n\
           <AudioInput>Disable</AudioInput>\r\n\
           <VideoInput>Disable</VideoInput>\r\n\
           <PrinterRedirection>Disable</PrinterRedirection>\r\n\
           <ClipboardRedirection>Disable</ClipboardRedirection>\r\n\
           <MemoryInMB>2048</MemoryInMB>\r\n\
           <MappedFolders>\r\n\
             <MappedFolder><HostFolder>{}</HostFolder><SandboxFolder>C:\\OpenMindWorkspace</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>\r\n\
             <MappedFolder><HostFolder>{}</HostFolder><SandboxFolder>C:\\OpenMindControl</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>\r\n\
           </MappedFolders>\r\n\
           <LogonCommand><Command>powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File C:\\OpenMindControl\\launcher.ps1</Command></LogonCommand>\r\n\
         </Configuration>\r\n",
        xml_escape(&display_path(workspace_root)),
        xml_escape(&display_path(&control_dir))
    );
    fs::write(&config_path, config.as_bytes())?;

    let started = Instant::now();
    let mut child = Command::new(executable)
        .arg(&config_path)
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| AppError::internal(format!("failed to start Windows Sandbox: {error}")))?;

    loop {
        if result_path.is_file() {
            break;
        }
        if let Some(status) = child.try_wait()? {
            let _ = fs::remove_dir_all(&control_dir);
            return Err(AppError::internal(format!(
                "Windows Sandbox exited before producing a command result (exit {})",
                status.code().unwrap_or(-1)
            )));
        }
        if started.elapsed() >= Duration::from_secs(timeout_secs) {
            let _ = child.kill().await;
            let _ = fs::remove_dir_all(&control_dir);
            return Ok(timeout_result(
                workspace_root,
                cwd,
                timeout_secs,
                started,
                "Windows Sandbox",
            ));
        }
        sleep(Duration::from_millis(200)).await;
    }

    let result_raw = fs::read_to_string(&result_path)?;
    let result_raw = result_raw.trim_start_matches('\u{feff}');
    let result: WindowsSandboxResult = serde_json::from_str(result_raw)
        .map_err(|error| AppError::internal(format!("invalid Windows Sandbox result: {error}")))?;
    let mut stdout = fs::read_to_string(&stdout_path).unwrap_or_default();
    let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
    let marker = take_cwd_marker(&mut stdout);
    let resolved_cwd = marker
        .as_deref()
        .and_then(|value| translate_windows_sandbox_cwd(workspace_root, value))
        .unwrap_or_else(|| display_path(cwd));
    let (stdout, stdout_truncated) = truncate_chars(&stdout, max_output_chars);
    let (stderr, stderr_truncated) = truncate_chars(&stderr, max_output_chars);

    if tokio::time::timeout(Duration::from_secs(15), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
    let _ = fs::remove_dir_all(&control_dir);

    Ok(ShellExecutionResult {
        cwd: resolved_cwd,
        exit_code: result.exit_code,
        stdout,
        stderr,
        duration_ms: started.elapsed().as_millis(),
        timed_out: false,
        truncated: stdout_truncated || stderr_truncated,
        backend: "Windows Sandbox".to_string(),
        isolated: true,
        network_disabled: true,
    })
}

#[cfg(not(target_os = "windows"))]
async fn run_windows_sandbox(
    _workspace_root: &Path,
    _cwd: &Path,
    _command: &str,
    _timeout_secs: u64,
    _max_output_chars: usize,
) -> Result<ShellExecutionResult, AppError> {
    Err(AppError::internal(
        "Windows Sandbox backend is only available on Windows",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct IsolatedOutput<'a> {
    workspace_root: &'a Path,
    cwd: &'a Path,
    exit_code: i32,
    stdout: &'a [u8],
    stderr: &'a [u8],
    started: Instant,
    max_output_chars: usize,
    backend: &'a str,
    sandbox_workspace_prefix: Option<&'a str>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn finish_isolated_output(output: IsolatedOutput<'_>) -> Result<ShellExecutionResult, AppError> {
    let IsolatedOutput {
        workspace_root,
        cwd,
        exit_code,
        stdout,
        stderr,
        started,
        max_output_chars,
        backend,
        sandbox_workspace_prefix,
    } = output;
    let mut stdout = String::from_utf8_lossy(stdout).into_owned();
    let stderr = String::from_utf8_lossy(stderr).into_owned();
    let marker = take_cwd_marker(&mut stdout);
    let resolved_cwd = marker
        .as_deref()
        .and_then(|value| {
            sandbox_workspace_prefix
                .and_then(|prefix| translate_sandbox_cwd(workspace_root, prefix, value))
                .or_else(|| canonical_scoped_marker(workspace_root, value))
        })
        .unwrap_or_else(|| display_path(cwd));
    let (stdout, stdout_truncated) = truncate_chars(&stdout, max_output_chars);
    let (stderr, stderr_truncated) = truncate_chars(&stderr, max_output_chars);
    Ok(ShellExecutionResult {
        cwd: resolved_cwd,
        exit_code,
        stdout,
        stderr,
        duration_ms: started.elapsed().as_millis(),
        timed_out: false,
        truncated: stdout_truncated || stderr_truncated,
        backend: backend.to_string(),
        isolated: true,
        network_disabled: true,
    })
}

fn timeout_result(
    _workspace_root: &Path,
    cwd: &Path,
    timeout_secs: u64,
    started: Instant,
    backend: &str,
) -> ShellExecutionResult {
    ShellExecutionResult {
        cwd: display_path(cwd),
        exit_code: -1,
        stdout: String::new(),
        stderr: format!("Command timed out after {timeout_secs} seconds."),
        duration_ms: started.elapsed().as_millis(),
        timed_out: true,
        truncated: false,
        backend: backend.to_string(),
        isolated: true,
        network_disabled: true,
    }
}

fn host_shell_process(command: &str, cwd: &Path) -> Command {
    #[cfg(target_os = "windows")]
    {
        let wrapped = powershell_wrapper(command);
        let mut process = Command::new("powershell.exe");
        process
            .arg("-NoLogo")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(wrapped)
            .current_dir(cwd);
        process
    }

    #[cfg(not(target_os = "windows"))]
    {
        let wrapped = shell_wrapper(command);
        let mut process = Command::new("/bin/sh");
        process.arg("-lc").arg(wrapped).current_dir(cwd);
        process
    }
}

#[cfg(not(target_os = "windows"))]
fn shell_wrapper(command: &str) -> String {
    format!(
        "{{ {command}; }}; openmind_code=$?; printf '\\n{RUNTIME_CWD_MARKER}%s\\n' \"$PWD\"; exit $openmind_code"
    )
}

#[cfg(target_os = "windows")]
fn powershell_wrapper(command: &str) -> String {
    format!(
        "& {{ {command}; $openmindCode = if ($null -ne $LASTEXITCODE) {{ [int]$LASTEXITCODE }} elseif ($?) {{ 0 }} else {{ 1 }}; Write-Output \"{RUNTIME_CWD_MARKER}$((Get-Location).Path)\"; exit $openmindCode }}"
    )
}

#[cfg(target_os = "windows")]
fn powershell_encoded_command(command: &str) -> String {
    let script = powershell_wrapper(command);
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    BASE64.encode(bytes)
}

fn take_cwd_marker(stdout: &mut String) -> Option<String> {
    let index = stdout.rfind(RUNTIME_CWD_MARKER)?;
    let cwd = stdout[index + RUNTIME_CWD_MARKER.len()..]
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    stdout.truncate(index);
    while stdout.ends_with('\r') || stdout.ends_with('\n') {
        stdout.pop();
    }
    (!cwd.is_empty()).then_some(cwd)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn translate_sandbox_cwd(workspace_root: &Path, prefix: &str, value: &str) -> Option<String> {
    let normalized = value.replace('\\', "/");
    if normalized == prefix {
        return Some(display_path(workspace_root));
    }
    let relative = normalized.strip_prefix(&format!("{prefix}/"))?;
    Some(display_path(&workspace_root.join(relative)))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn canonical_scoped_marker(workspace_root: &Path, value: &str) -> Option<String> {
    let candidate = fs::canonicalize(value).ok()?;
    candidate
        .starts_with(workspace_root)
        .then(|| display_path(&candidate))
}

#[cfg(target_os = "windows")]
fn translate_windows_sandbox_cwd(workspace_root: &Path, value: &str) -> Option<String> {
    let normalized = value.replace('/', "\\");
    let prefix = "C:\\OpenMindWorkspace";
    if normalized.eq_ignore_ascii_case(prefix) {
        return Some(display_path(workspace_root));
    }
    let expected = format!("{prefix}\\");
    if normalized.len() < expected.len()
        || !normalized[..expected.len()].eq_ignore_ascii_case(&expected)
    {
        return None;
    }
    let relative = &normalized[expected.len()..];
    Some(display_path(&workspace_root.join(relative)))
}

#[cfg(target_os = "macos")]
fn sandbox_profile_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(target_os = "windows")]
fn powershell_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(target_os = "windows")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn truncate_chars(value: &str, limit: usize) -> (String, bool) {
    let mut chars = value.chars();
    let output = chars.by_ref().take(limit).collect::<String>();
    (output, chars.next().is_some())
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(target_os = "windows")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowsSandboxResult {
    exit_code: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_is_removed_from_stdout() {
        let mut stdout = format!("hello\n{RUNTIME_CWD_MARKER}/workspace/src\n");
        let cwd = take_cwd_marker(&mut stdout).unwrap();
        assert_eq!(cwd, "/workspace/src");
        assert_eq!(stdout, "hello");
    }

    #[test]
    fn isolated_scope_rejects_cwd_outside_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&outside).unwrap();
        assert!(validate_isolated_scope(&workspace, &outside).is_err());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn sandbox_cwd_translation_stays_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(workspace.join("src")).unwrap();
        assert_eq!(
            translate_sandbox_cwd(&workspace, "/workspace", "/workspace/src"),
            Some(display_path(&workspace.join("src")))
        );
        assert!(translate_sandbox_cwd(&workspace, "/workspace", "/other/src").is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_sandbox_cwd_translation_stays_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(workspace.join("src")).unwrap();
        assert_eq!(
            translate_windows_sandbox_cwd(&workspace, r"C:\OpenMindWorkspace\src"),
            Some(display_path(&workspace.join("src")))
        );
        assert!(translate_windows_sandbox_cwd(&workspace, r"C:\OtherWorkspace\src").is_none());
    }

    #[test]
    fn capability_message_is_always_actionable() {
        let capability = sandbox_capability();
        assert!(!capability.platform.is_empty());
        assert!(!capability.message.is_empty());
        assert_eq!(capability.available, capability.strong_isolation);
    }
}
