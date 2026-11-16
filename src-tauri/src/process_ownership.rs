//! Operating-system ownership of the child processes OpenMindAI starts.
//!
//! Model servers must not outlive the app, including when it crashes or is force-killed
//! (Task Manager, `taskkill /F`), where no shutdown code runs.
//!
//! - Windows: every owned child joins one Job Object created with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Only this process holds the job handle, so when
//!   the process ends for any reason the OS closes the handle and terminates every process
//!   in the job.
//! - Linux: long-lived children are spawned from one dedicated thread with
//!   `PR_SET_PDEATHSIG = SIGKILL`. The death signal follows the spawning *thread*, so
//!   spawning from tokio's short-lived blocking threads would kill model servers early.
//! - macOS: isolated process groups and identity-checked startup recovery. Unlike the
//!   Windows/Linux mechanisms, a crash can leave runtimes alive until the next launch.
//! - Other platforms: normal-exit cleanup only; unknown process identities are never killed.
//!
//! Long-lived runtimes are also recorded with their creation time and executable path so a
//! later start can remove leftovers (for example from a platform without OS ownership)
//! without ever terminating a process it cannot prove it started: a matching PID alone is
//! never enough, because PIDs are reused.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

/// Identity that survives PID reuse: creation time and executable must also match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessIdentity {
    pub pid: u32,
    /// Platform start time (Windows FILETIME ticks, Linux clock ticks, macOS microseconds).
    pub started: u64,
    pub executable: String,
    /// Darwin's process generation, checked by the kernel when signalling an orphan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos_pid_version: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnedRecord {
    owner: ProcessIdentity,
    child: ProcessIdentity,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Leftover runtimes whose owner is gone and whose identity matched exactly.
    pub terminated: Vec<u32>,
    /// Records retained for a live/uninspectable owner or a failed termination.
    pub kept: usize,
    /// Records dropped because the process no longer exists or is a different process.
    pub dropped: usize,
}

static REGISTRY_LOCK: Mutex<()> = Mutex::new(());

/// Spawns `command` with the platform's ownership/recovery isolation.
pub fn spawn_owned(command: Command) -> io::Result<Child> {
    let child = platform::spawn(command)?;
    platform::adopt(&child);
    Ok(child)
}

/// Adds an already spawned tokio child (bounded helper processes) to the ownership job.
/// On Linux these keep their existing process-group and kill-on-drop handling.
pub fn adopt_tokio_child(child: &tokio::process::Child) {
    #[cfg(target_os = "windows")]
    if let Some(handle) = child.raw_handle() {
        platform::adopt_raw(handle);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = child;
}

/// `Command::output` for bounded helper processes, with the child owned like a runtime.
pub async fn output_owned(
    mut command: tokio::process::Command,
) -> io::Result<std::process::Output> {
    let child = command.spawn()?;
    adopt_tokio_child(&child);
    child.wait_with_output().await
}

const MAX_RENDERER_LOG_BYTES: usize = 32 * 1024;
const MAX_RENDERER_LINE_BYTES: usize = 2048;

/// Collect bounded diagnostic output while forwarding complete progress
/// updates. Dropping this future cancels a kill-on-drop renderer child.
pub async fn output_owned_with_progress_lines(
    mut command: tokio::process::Command,
    on_line: &(dyn Fn(&str) + Send + Sync),
) -> io::Result<std::process::Output> {
    let mut child = command.spawn()?;
    adopt_tokio_child(&child);
    let stdout = child.stdout.take().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "renderer stdout must be piped")
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "renderer stderr must be piped")
    })?;
    let (status, stdout, stderr) = tokio::try_join!(
        child.wait(),
        capture_renderer_stream(stdout, Some(on_line)),
        capture_renderer_stream(stderr, Some(on_line)),
    )?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

async fn capture_renderer_stream<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    on_line: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;

    let mut captured = Vec::new();
    let mut partial_line = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            break;
        }
        captured.extend_from_slice(&chunk[..count]);
        if captured.len() > MAX_RENDERER_LOG_BYTES {
            captured.drain(..captured.len() - MAX_RENDERER_LOG_BYTES);
        }
        if let Some(callback) = on_line {
            for &byte in &chunk[..count] {
                if byte == b'\r' || byte == b'\n' {
                    if !partial_line.is_empty() {
                        callback(&String::from_utf8_lossy(&partial_line));
                        partial_line.clear();
                    }
                } else if partial_line.len() < MAX_RENDERER_LINE_BYTES {
                    partial_line.push(byte);
                } else {
                    partial_line.clear();
                }
            }
        }
    }
    if let Some(callback) = on_line {
        if !partial_line.is_empty() {
            callback(&String::from_utf8_lossy(&partial_line));
        }
    }
    Ok(captured)
}

/// Identity of a running process, or `None` if it does not exist or cannot be inspected.
pub fn identity(pid: u32) -> Option<ProcessIdentity> {
    platform::identity(pid)
}

pub fn current_identity() -> Option<ProcessIdentity> {
    identity(std::process::id())
}

/// Records a runtime child in `registry` so a later start can prove ownership.
pub fn record(registry: &Path, child_pid: u32) {
    let (Some(owner), Some(child)) = (current_identity(), identity(child_pid)) else {
        return;
    };
    let _guard = REGISTRY_LOCK.lock();
    let mut records = read_records(registry);
    records.retain(|record| record.child.pid != child_pid);
    records.push(OwnedRecord { owner, child });
    write_records(registry, &records);
}

/// Removes a runtime child from `registry` after it was stopped.
pub fn forget(registry: &Path, child_pid: u32) {
    let _guard = REGISTRY_LOCK.lock();
    let mut records = read_records(registry);
    let before = records.len();
    records.retain(|record| record.child.pid != child_pid);
    if records.len() != before {
        write_records(registry, &records);
    }
}

/// Terminates leftover runtimes from instances that are no longer running. A process is
/// only terminated when its PID, creation time and executable all match the record and the
/// recorded owner (PID + creation time) is gone.
pub fn sweep_stale(registry: &Path) -> SweepReport {
    sweep_with(
        registry,
        identity,
        process_may_exist,
        platform::terminate_if_identity,
    )
}

fn process_may_exist(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        if pid <= 0 {
            return false;
        }
        // SAFETY: signal 0 only probes existence/permissions; it sends no signal.
        unsafe {
            libc::kill(pid, 0) == 0
                || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        }
    }
    #[cfg(target_os = "windows")]
    {
        platform::process_may_exist(pid)
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = pid;
        true
    }
}

fn sweep_with(
    registry: &Path,
    lookup: impl Fn(u32) -> Option<ProcessIdentity>,
    may_exist: impl Fn(u32) -> bool,
    terminate: impl Fn(&ProcessIdentity) -> bool,
) -> SweepReport {
    let _guard = REGISTRY_LOCK.lock();
    let mut report = SweepReport::default();
    let mut kept = Vec::new();
    for record in read_records(registry) {
        let owner = lookup(record.owner.pid);
        if owner.as_ref() == Some(&record.owner) || (owner.is_none() && may_exist(record.owner.pid))
        {
            report.kept += 1;
            kept.push(record);
            continue;
        }
        let child = lookup(record.child.pid);
        if child.as_ref() == Some(&record.child) {
            if terminate(&record.child) {
                report.terminated.push(record.child.pid);
            } else {
                // A permission/symbol failure must not discard our only recovery record.
                report.kept += 1;
                kept.push(record);
            }
        } else if child.is_none() && may_exist(record.child.pid) {
            report.kept += 1;
            kept.push(record);
        } else {
            report.dropped += 1;
        }
    }
    write_records(registry, &kept);
    report
}

pub fn registry_path(root_runtimes_dir: &Path) -> PathBuf {
    root_runtimes_dir.join("owned-processes.json")
}

fn read_records(registry: &Path) -> Vec<OwnedRecord> {
    fs::read(registry)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_records(registry: &Path, records: &[OwnedRecord]) {
    if records.is_empty() {
        let _ = fs::remove_file(registry);
        return;
    }
    if let Some(parent) = registry.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let temp = registry.with_extension(format!("json.{}.tmp", std::process::id()));
    if fs::write(
        &temp,
        serde_json::to_vec_pretty(records).unwrap_or_default(),
    )
    .is_ok()
        && fs::rename(&temp, registry).is_err()
    {
        let _ = fs::remove_file(&temp);
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{
        io,
        os::windows::io::AsRawHandle,
        process::{Child, Command},
        sync::OnceLock,
    };

    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, HANDLE},
            System::{
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                },
                Threading::{
                    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
                    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
                },
            },
        },
    };

    use super::ProcessIdentity;

    /// Raw job handle value. Deliberately never closed: closing it is what kills the job,
    /// and the OS does that when this process ends.
    struct Job(isize);

    fn job() -> Option<HANDLE> {
        static JOB: OnceLock<Option<Job>> = OnceLock::new();
        JOB.get_or_init(|| {
            // SAFETY: plain Win32 calls with valid arguments; the handle is owned by `Job`.
            unsafe {
                let handle = CreateJobObjectW(None, None).ok()?;
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .is_err()
                {
                    let _ = CloseHandle(handle);
                    return None;
                }
                Some(Job(handle.0 as isize))
            }
        })
        .as_ref()
        .map(|job| HANDLE(job.0 as *mut core::ffi::c_void))
    }

    pub fn spawn(mut command: Command) -> io::Result<Child> {
        command.spawn()
    }

    pub fn adopt(child: &Child) {
        adopt_raw(child.as_raw_handle());
    }

    pub fn adopt_raw(raw: std::os::windows::io::RawHandle) {
        let Some(job) = job() else {
            tracing::warn!("process ownership job unavailable; child may outlive OpenMindAI");
            return;
        };
        // SAFETY: `raw` is a live process handle owned by the caller's Child.
        if let Err(error) = unsafe { AssignProcessToJobObject(job, HANDLE(raw)) } {
            tracing::warn!(%error, "could not add child process to the ownership job");
        }
    }

    pub fn identity(pid: u32) -> Option<ProcessIdentity> {
        // SAFETY: the handle is closed before returning.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let identity = identity_of(handle, pid);
            let _ = CloseHandle(handle);
            identity
        }
    }

    pub fn process_may_exist(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        // SAFETY: this only opens a query handle; failure other than an invalid PID
        // (notably access denied) is not evidence that an owner is dead.
        unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(handle) => {
                    let _ = CloseHandle(handle);
                    true
                }
                Err(error) => {
                    error.code() != windows::core::HRESULT::from_win32(ERROR_INVALID_PARAMETER.0)
                }
            }
        }
    }

    unsafe fn identity_of(handle: HANDLE, pid: u32) -> Option<ProcessIdentity> {
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
            .ok()?;
        let mut buffer = vec![0u16; 32_768];
        let mut length = buffer.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        }
        .ok()?;
        Some(ProcessIdentity {
            pid,
            started: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
            executable: String::from_utf16_lossy(&buffer[..length as usize]),
            macos_pid_version: None,
        })
    }

    /// Re-checks the identity on the same handle used to terminate, so a PID reused between
    /// the check and the kill can never be hit.
    pub fn terminate_if_identity(expected: &ProcessIdentity) -> bool {
        // SAFETY: the handle is closed before returning.
        unsafe {
            let Ok(handle) = OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                expected.pid,
            ) else {
                return false;
            };
            let matches = identity_of(handle, expected.pid).as_ref() == Some(expected);
            let terminated = matches && TerminateProcess(handle, 1).is_ok();
            let _ = CloseHandle(handle);
            terminated
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::{
        fs, io,
        os::unix::process::CommandExt,
        process::{Child, Command},
        sync::{mpsc, Mutex, OnceLock},
    };

    use super::ProcessIdentity;

    type Request = (Command, mpsc::Sender<io::Result<Child>>);

    /// All owned children are spawned from this thread, which lives as long as the process,
    /// so PR_SET_PDEATHSIG fires only when OpenMindAI itself dies.
    fn spawner() -> &'static Mutex<mpsc::Sender<Request>> {
        static SPAWNER: OnceLock<Mutex<mpsc::Sender<Request>>> = OnceLock::new();
        SPAWNER.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<Request>();
            std::thread::Builder::new()
                .name("openmindai-process-owner".to_string())
                .spawn(move || {
                    for (mut command, reply) in receiver {
                        let _ = reply.send(command.spawn());
                    }
                })
                .expect("failed to start the process owner thread");
            Mutex::new(sender)
        })
    }

    pub fn spawn(mut command: Command) -> io::Result<Child> {
        let parent = std::process::id() as libc::pid_t;
        // SAFETY: only async-signal-safe libc calls run between fork and exec.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(io::Error::last_os_error());
                }
                // The parent may have died before prctl took effect.
                if libc::getppid() != parent {
                    return Err(io::Error::other("parent exited during spawn"));
                }
                Ok(())
            });
        }
        let (reply, result) = mpsc::channel();
        spawner()
            .lock()
            .map_err(|_| io::Error::other("process owner thread lock poisoned"))?
            .send((command, reply))
            .map_err(|_| io::Error::other("process owner thread stopped"))?;
        result
            .recv()
            .map_err(|_| io::Error::other("process owner thread stopped"))?
    }

    pub fn adopt(_child: &Child) {}

    pub fn identity(pid: u32) -> Option<ProcessIdentity> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // Fields after the parenthesized command name; starttime is field 22 overall.
        let rest = stat.rsplit_once(')')?.1;
        let started = rest.split_whitespace().nth(19)?.parse().ok()?;
        let executable = fs::read_link(format!("/proc/{pid}/exe"))
            .ok()?
            .display()
            .to_string();
        Some(ProcessIdentity {
            pid,
            started,
            executable,
            macos_pid_version: None,
        })
    }

    pub fn terminate_if_identity(expected: &ProcessIdentity) -> bool {
        if identity(expected.pid).as_ref() != Some(expected) {
            return false;
        }
        // SAFETY: plain kill(2) on a PID whose identity was just verified.
        unsafe { libc::kill(expected.pid as libc::pid_t, libc::SIGKILL) == 0 }
    }
}

#[cfg(target_os = "macos")]
#[path = "process_ownership/macos.rs"]
mod platform;

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod platform {
    use std::{
        io,
        process::{Child, Command},
    };

    use super::ProcessIdentity;

    pub fn spawn(mut command: Command) -> io::Result<Child> {
        command.spawn()
    }

    pub fn adopt(_child: &Child) {}

    /// Without a safe identity source nothing is ever terminated by the sweep.
    pub fn identity(_pid: u32) -> Option<ProcessIdentity> {
        None
    }

    pub fn terminate_if_identity(_expected: &ProcessIdentity) -> bool {
        false
    }
}

#[cfg(all(
    test,
    any(target_os = "windows", target_os = "linux", target_os = "macos")
))]
mod tests {
    use super::*;
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };

    use std::io::{BufRead, BufReader};

    const HELPER_ENV: &str = "OPENMINDAI_OWNERSHIP_HELPER";
    const REGISTRY_ENV: &str = "OPENMINDAI_OWNERSHIP_REGISTRY";
    const SLEEPER_ENV: &str = "OPENMINDAI_OWNERSHIP_SLEEPER";

    /// Re-runs this test binary as a helper process for one ignored test.
    fn self_command(test: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            test,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ]);
        command
    }

    fn sleeper() -> Command {
        let mut command = self_command("process_ownership::tests::sleeper_child");
        command
            .env(SLEEPER_ENV, "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn alive(pid: u32) -> bool {
        identity(pid).is_some()
    }

    fn wait_until_gone(pid: u32, timeout: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if !alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        !alive(pid)
    }

    /// Stand-in model server: idles until killed.
    #[test]
    #[ignore = "helper process for ownership tests"]
    fn sleeper_child() {
        if std::env::var(SLEEPER_ENV).is_ok() {
            std::thread::sleep(Duration::from_secs(600));
        }
    }

    /// Stand-in OpenMindAI: starts two owned "runtimes" (Core + Agent) through the real
    /// ownership path, prints their PIDs, then either waits to be killed or exits without
    /// running any cleanup.
    #[test]
    #[ignore = "helper process for ownership tests"]
    // Deliberately never waits on its children: the test checks the OS removes them.
    #[allow(clippy::zombie_processes)]
    fn owner_parent() {
        let Ok(mode) = std::env::var(HELPER_ENV) else {
            return;
        };
        let core = spawn_owned(sleeper()).unwrap();
        let agent = spawn_owned(sleeper()).unwrap();
        if let Some(path) = std::env::var_os(REGISTRY_ENV) {
            record(Path::new(&path), core.id());
            record(Path::new(&path), agent.id());
        }
        println!("CHILDREN {} {}", core.id(), agent.id());
        if mode == "exit" {
            std::thread::sleep(Duration::from_millis(500));
            // Leave without dropping or killing the children.
            std::mem::forget((core, agent));
            std::process::exit(0);
        }
        std::thread::sleep(Duration::from_secs(600));
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn start_owner(mode: &str) -> (Child, Vec<u32>) {
        start_owner_with_registry(mode, None)
    }

    fn start_owner_with_registry(mode: &str, registry: Option<&Path>) -> (Child, Vec<u32>) {
        let mut command = self_command("process_ownership::tests::owner_parent");
        command
            .env(HELPER_ENV, mode)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(registry) = registry {
            command.env(REGISTRY_ENV, registry);
        }
        let mut parent = command.spawn().unwrap();
        let stdout = parent.stdout.take().unwrap();
        let line = BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            // libtest prints "test <name> ... " on the same line first.
            .find_map(|line| {
                line.split_once("CHILDREN ")
                    .map(|(_, pids)| pids.to_string())
            })
            .expect("owner did not report its children");
        let pids = line
            .split_whitespace()
            .map(|pid| pid.parse().unwrap())
            .collect();
        (parent, pids)
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn force_killed_owner_takes_its_runtimes_with_it() {
        // An unrelated process that looks the same must survive.
        let mut unrelated = sleeper().spawn().unwrap();
        let (mut parent, children) = start_owner("wait");
        assert_eq!(children.len(), 2);
        assert!(children.iter().all(|pid| alive(*pid)));

        // TerminateProcess / SIGKILL: no destructor or exit handler of the owner runs.
        parent.kill().unwrap();
        let _ = parent.wait();

        for pid in children {
            assert!(
                wait_until_gone(pid, Duration::from_secs(10)),
                "owned child {pid} survived"
            );
        }
        assert!(alive(unrelated.id()), "unrelated process was terminated");
        unrelated.kill().unwrap();
        let _ = unrelated.wait();
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn owner_exiting_without_cleanup_takes_its_runtimes_with_it() {
        let (mut parent, children) = start_owner("exit");
        let _ = parent.wait();
        for pid in children {
            assert!(
                wait_until_gone(pid, Duration::from_secs(10)),
                "owned child {pid} survived"
            );
        }
    }

    #[test]
    fn identity_is_stable_and_missing_for_dead_processes() {
        let me = current_identity().expect("own identity");
        assert_eq!(identity(me.pid), Some(me.clone()));
        assert!(me.started > 0);
        let mut child = sleeper().spawn().unwrap();
        let pid = child.id();
        let before = identity(pid).unwrap();
        child.kill().unwrap();
        let _ = child.wait();
        assert!(wait_until_gone(pid, Duration::from_secs(5)));
        assert_ne!(identity(pid), Some(before));
    }

    fn record_for(owner: ProcessIdentity, child: ProcessIdentity) -> OwnedRecord {
        OwnedRecord { owner, child }
    }

    #[test]
    fn sweep_only_terminates_proven_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_path(dir.path());
        let me = current_identity().unwrap();
        let live = ProcessIdentity {
            pid: 4242,
            started: 100,
            executable: "C:/runtimes/llama-server.exe".to_string(),
            macos_pid_version: None,
        };
        let dead_owner = ProcessIdentity {
            pid: 999_991,
            started: 1,
            executable: "openmindai".to_string(),
            macos_pid_version: None,
        };
        let records = vec![
            // 1. Owner is still running (another instance): never touched.
            record_for(me.clone(), live.clone()),
            // 2. Dead owner, matching child identity: a proven leftover.
            record_for(dead_owner.clone(), live.clone()),
            // 3. Dead owner, PID reused by a process with another start time.
            record_for(
                dead_owner.clone(),
                ProcessIdentity {
                    pid: 5555,
                    started: 7,
                    executable: "C:/runtimes/llama-server.exe".to_string(),
                    macos_pid_version: None,
                },
            ),
            // 4. Dead owner, PID reused by a different executable.
            record_for(
                dead_owner.clone(),
                ProcessIdentity {
                    pid: 6666,
                    started: 9,
                    executable: "C:/runtimes/llama-server.exe".to_string(),
                    macos_pid_version: None,
                },
            ),
            // 5. Dead owner, process already gone.
            record_for(
                dead_owner,
                ProcessIdentity {
                    pid: 7777,
                    started: 3,
                    executable: "C:/runtimes/llama-server.exe".to_string(),
                    macos_pid_version: None,
                },
            ),
        ];
        write_records(&registry, &records);

        let world = |pid: u32| match pid {
            4242 => Some(live.clone()),
            5555 => Some(ProcessIdentity {
                pid,
                started: 8,
                executable: "C:/runtimes/llama-server.exe".to_string(),
                macos_pid_version: None,
            }),
            6666 => Some(ProcessIdentity {
                pid,
                started: 9,
                executable: "C:/other-app/llama-server.exe".to_string(),
                macos_pid_version: None,
            }),
            pid if pid == me.pid => Some(me.clone()),
            _ => None,
        };
        let killed = std::cell::RefCell::new(Vec::new());
        let report = sweep_with(
            &registry,
            world,
            |_| false,
            |identity| {
                killed.borrow_mut().push(identity.pid);
                true
            },
        );

        assert_eq!(*killed.borrow(), vec![4242]);
        assert_eq!(report.terminated, vec![4242]);
        assert_eq!(report.kept, 1);
        assert_eq!(report.dropped, 3);
        // Only the record of the live owner remains.
        assert_eq!(read_records(&registry), vec![records[0].clone()]);
    }

    #[test]
    fn sweep_with_real_processes_spares_unrelated_and_kills_proven_orphans() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_path(dir.path());
        let mut orphan = sleeper().spawn().unwrap();
        let mut unrelated = sleeper().spawn().unwrap();
        let orphan_identity = identity(orphan.id()).unwrap();
        let mut stale_unrelated = identity(unrelated.id()).unwrap();
        // Same PID and executable, but a different start time: what PID reuse looks like.
        stale_unrelated.started += 1;
        let dead_owner = ProcessIdentity {
            pid: 999_993,
            started: 1,
            executable: "openmindai".to_string(),
            macos_pid_version: None,
        };
        write_records(
            &registry,
            &[
                record_for(dead_owner.clone(), orphan_identity),
                record_for(dead_owner, stale_unrelated),
            ],
        );

        let report = sweep_stale(&registry);
        assert_eq!(report.terminated, vec![orphan.id()]);
        assert!(wait_until_gone(orphan.id(), Duration::from_secs(5)));
        assert!(alive(unrelated.id()), "unrelated process was terminated");
        let _ = orphan.wait();
        unrelated.kill().unwrap();
        let _ = unrelated.wait();
        assert!(read_records(&registry).is_empty());
    }

    #[test]
    fn record_and_forget_track_live_children() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_path(dir.path());
        let mut child = spawn_owned(sleeper()).unwrap();
        record(&registry, child.id());
        let records = read_records(&registry);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].child.pid, child.id());
        assert_eq!(Some(records[0].owner.clone()), current_identity());
        // Owner alive: the sweep keeps it.
        assert_eq!(sweep_stale(&registry).kept, 1);
        assert!(alive(child.id()));
        forget(&registry, child.id());
        assert!(read_records(&registry).is_empty());
        child.kill().unwrap();
        let _ = child.wait();
    }

    #[test]
    fn uninspectable_owners_and_children_are_never_killed_or_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_path(dir.path());
        let owner = ProcessIdentity {
            pid: 100,
            started: 1,
            executable: "owner".into(),
            macos_pid_version: None,
        };
        let child = ProcessIdentity {
            pid: 200,
            started: 2,
            executable: "runtime".into(),
            macos_pid_version: None,
        };
        let records = vec![record_for(owner, child.clone())];
        write_records(&registry, &records);
        let report = sweep_with(
            &registry,
            |pid| (pid == 200).then(|| child.clone()),
            |pid| pid == 100,
            |_| panic!("unproven owner was treated as dead"),
        );
        assert_eq!(report.kept, 1);
        assert_eq!(read_records(&registry), records);
        let report = sweep_with(
            &registry,
            |_| None,
            |pid| pid == 200,
            |_| panic!("uninspectable child was signalled"),
        );
        assert_eq!(report.kept, 1);
        assert_eq!(read_records(&registry), records);
    }

    #[test]
    fn failed_termination_is_retained_for_a_later_launch() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_path(dir.path());
        let owner = ProcessIdentity {
            pid: 100,
            started: 1,
            executable: "owner".into(),
            macos_pid_version: None,
        };
        let child = ProcessIdentity {
            pid: 200,
            started: 2,
            executable: "runtime".into(),
            macos_pid_version: Some(7),
        };
        let records = vec![record_for(owner, child.clone())];
        write_records(&registry, &records);
        let lookup = |pid| (pid == 200).then(|| child.clone());
        assert_eq!(sweep_with(&registry, lookup, |_| false, |_| false).kept, 1);
        assert_eq!(read_records(&registry), records);
        assert_eq!(
            sweep_with(&registry, lookup, |_| false, |_| true).terminated,
            vec![200]
        );
        assert!(read_records(&registry).is_empty());
    }

    #[test]
    fn existing_registry_json_without_darwin_generation_remains_readable() {
        let old: ProcessIdentity =
            serde_json::from_str(r#"{"pid":42,"started":7,"executable":"old-runtime"}"#).unwrap();
        assert_eq!(old.macos_pid_version, None);
        assert!(!serde_json::to_string(&old)
            .unwrap()
            .contains("macosPidVersion"));
    }

    #[test]
    #[ignore = "helper process for macOS relaunch tests"]
    fn relaunch_sweep() {
        if let Some(path) = std::env::var_os(REGISTRY_ENV) {
            let report = sweep_stale(Path::new(&path));
            println!("REAPED {:?}", report.terminated);
        }
    }

    #[cfg(target_os = "macos")]
    fn verify_macos_relaunch(mode: &str) {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_path(dir.path());
        let mut unrelated = sleeper().spawn().unwrap();
        let (mut owner, children) = start_owner_with_registry(mode, Some(&registry));
        let records = read_records(&registry);
        assert_eq!(records.len(), 2, "Darwin identities were not recorded");
        for record in &records {
            assert!(record.owner.macos_pid_version.is_some());
            assert!(record.child.macos_pid_version.is_some());
            // SAFETY: getpgid only inspects this test's child process.
            assert_eq!(
                unsafe { libc::getpgid(record.child.pid as i32) },
                record.child.pid as i32
            );
        }
        if mode == "wait" {
            owner.kill().unwrap();
        }
        let _ = owner.wait();
        // Darwin recovery is on the next launch, not an immediate death signal.
        assert!(children.iter().all(|pid| alive(*pid)));
        let output = self_command("process_ownership::tests::relaunch_sweep")
            .env(REGISTRY_ENV, &registry)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = String::from_utf8(output.stdout).unwrap();
        for pid in children {
            assert!(
                output.contains(&pid.to_string()),
                "relaunch did not reap {pid}: {output}"
            );
            assert!(
                wait_until_gone(pid, Duration::from_secs(5)),
                "orphan {pid} survived relaunch"
            );
        }
        assert!(alive(unrelated.id()), "unrelated runtime was killed");
        assert!(read_records(&registry).is_empty());
        unrelated.kill().unwrap();
        let _ = unrelated.wait();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_relaunch_recovers_runtimes_after_force_killed_owner() {
        verify_macos_relaunch("wait");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_relaunch_recovers_runtimes_after_owner_exits_without_cleanup() {
        verify_macos_relaunch("exit");
    }
}

#[cfg(test)]
mod renderer_output_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn renderer_lines_handle_carriage_returns_and_split_writes() {
        let (mut writer, reader) = tokio::io::duplex(128);
        let write_task = tokio::spawn(async move {
            writer
                .write_all(b"Sampling step 1/12\rSampling step ")
                .await
                .unwrap();
            writer.write_all(b"2/12\ncompleted").await.unwrap();
        });
        let lines = Mutex::new(Vec::<String>::new());
        let callback = |line: &str| lines.lock().unwrap().push(line.to_string());
        let bytes = capture_renderer_stream(reader, Some(&callback))
            .await
            .unwrap();
        write_task.await.unwrap();
        assert_eq!(bytes, b"Sampling step 1/12\rSampling step 2/12\ncompleted");
        assert_eq!(
            *lines.lock().unwrap(),
            vec!["Sampling step 1/12", "Sampling step 2/12", "completed",]
        );
    }

    #[tokio::test]
    async fn renderer_output_memory_is_bounded() {
        let (mut writer, reader) = tokio::io::duplex(8192);
        let write_task = tokio::spawn(async move {
            writer
                .write_all(&vec![b'x'; MAX_RENDERER_LOG_BYTES + 1024])
                .await
                .unwrap();
        });
        let bytes = capture_renderer_stream(reader, None).await.unwrap();
        write_task.await.unwrap();
        assert_eq!(bytes.len(), MAX_RENDERER_LOG_BYTES);
        assert!(bytes.iter().all(|byte| *byte == b'x'));
    }
}
