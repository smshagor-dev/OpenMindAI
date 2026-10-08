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
//! - Other platforms: no OS mechanism; normal-exit cleanup and the startup sweep apply.
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
    /// Platform start time (Windows FILETIME ticks, Linux clock ticks since boot).
    pub started: u64,
    pub executable: String,
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
    /// Records kept because their owner is still running (another OpenMindAI instance).
    pub kept: usize,
    /// Records dropped because the process no longer exists or is a different process.
    pub dropped: usize,
}

static REGISTRY_LOCK: Mutex<()> = Mutex::new(());

/// Spawns `command` so the child cannot outlive this process.
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
    sweep_with(registry, identity, platform::terminate_if_identity)
}

fn sweep_with(
    registry: &Path,
    lookup: impl Fn(u32) -> Option<ProcessIdentity>,
    terminate: impl Fn(&ProcessIdentity) -> bool,
) -> SweepReport {
    let _guard = REGISTRY_LOCK.lock();
    let mut report = SweepReport::default();
    let mut kept = Vec::new();
    for record in read_records(registry) {
        if lookup(record.owner.pid).as_ref() == Some(&record.owner) {
            report.kept += 1;
            kept.push(record);
            continue;
        }
        if lookup(record.child.pid).as_ref() == Some(&record.child) && terminate(&record.child) {
            report.terminated.push(record.child.pid);
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
            Foundation::{CloseHandle, FILETIME, HANDLE},
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

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        io::{BufRead, BufReader},
        process::Stdio,
        time::{Duration, Instant},
    };

    const HELPER_ENV: &str = "OPENMINDAI_OWNERSHIP_HELPER";
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
        let mut parent = self_command("process_ownership::tests::owner_parent")
            .env(HELPER_ENV, mode)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
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
        };
        let dead_owner = ProcessIdentity {
            pid: 999_991,
            started: 1,
            executable: "openmindai".to_string(),
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
                },
            ),
            // 4. Dead owner, PID reused by a different executable.
            record_for(
                dead_owner.clone(),
                ProcessIdentity {
                    pid: 6666,
                    started: 9,
                    executable: "C:/runtimes/llama-server.exe".to_string(),
                },
            ),
            // 5. Dead owner, process already gone.
            record_for(
                dead_owner,
                ProcessIdentity {
                    pid: 7777,
                    started: 3,
                    executable: "C:/runtimes/llama-server.exe".to_string(),
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
            }),
            6666 => Some(ProcessIdentity {
                pid,
                started: 9,
                executable: "C:/other-app/llama-server.exe".to_string(),
            }),
            pid if pid == me.pid => Some(me.clone()),
            _ => None,
        };
        let killed = std::cell::RefCell::new(Vec::new());
        let report = sweep_with(&registry, world, |identity| {
            killed.borrow_mut().push(identity.pid);
            true
        });

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
}
