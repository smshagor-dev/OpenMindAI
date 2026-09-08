use std::{io, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
};

use crate::app_error::AppError;

const ISOLATED_MEMORY_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const ISOLATED_OPEN_FILES: u64 = 1_024;
const ISOLATED_PROCESSES: u64 = 256;
const ISOLATED_FILE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Debug)]
pub struct ProcessCapture {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    pub truncated: bool,
}

#[derive(Debug)]
struct CappedBytes {
    bytes: Vec<u8>,
    truncated: bool,
}

#[cfg(unix)]
pub fn resource_limit_labels() -> Vec<String> {
    vec![
        "CPU time bounded relative to the command timeout".to_string(),
        "address space <= 8 GiB".to_string(),
        "open files <= 1024".to_string(),
        "processes <= 256".to_string(),
        "single file <= 8 GiB".to_string(),
    ]
}

#[cfg(target_os = "windows")]
pub fn resource_limit_labels() -> Vec<String> {
    vec![
        "Windows Sandbox memory <= 4096 MiB".to_string(),
        "wall-clock timeout with process-tree termination".to_string(),
        "bounded stdout/stderr capture".to_string(),
    ]
}

#[cfg(not(any(unix, target_os = "windows")))]
pub fn resource_limit_labels() -> Vec<String> {
    vec!["wall-clock timeout with bounded output capture".to_string()]
}

pub fn apply_isolated_limits(command: &mut Command, timeout_secs: u64) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;

        let cpu_seconds = timeout_secs.saturating_mul(2).clamp(30, 1_200) as libc::rlim_t;
        let memory = ISOLATED_MEMORY_BYTES as libc::rlim_t;
        let open_files = ISOLATED_OPEN_FILES as libc::rlim_t;
        let processes = ISOLATED_PROCESSES as libc::rlim_t;
        let file_bytes = ISOLATED_FILE_BYTES as libc::rlim_t;

        // SAFETY: pre_exec runs after fork and before exec. The closure only invokes
        // async-signal-safe setrlimit calls and constructs an io::Error from errno.
        unsafe {
            command.as_std_mut().pre_exec(move || {
                set_limit(libc::RLIMIT_CPU, cpu_seconds)?;
                set_limit(libc::RLIMIT_AS, memory)?;
                set_limit(libc::RLIMIT_NOFILE, open_files)?;
                set_limit(libc::RLIMIT_NPROC, processes)?;
                set_limit(libc::RLIMIT_FSIZE, file_bytes)?;
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (command, timeout_secs);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
unsafe fn set_limit(resource: libc::__rlimit_resource_t, value: libc::rlim_t) -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    if unsafe { libc::setrlimit(resource, &limit) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
unsafe fn set_limit(resource: libc::c_int, value: libc::rlim_t) -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    if unsafe { libc::setrlimit(resource, &limit) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub async fn run_process(
    mut command: Command,
    timeout_secs: u64,
    max_output_chars: usize,
) -> Result<ProcessCapture, AppError> {
    prepare_process_tree(&mut command);
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::internal("process stdout pipe was not created"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::internal("process stderr pipe was not created"))?;
    let byte_limit = max_output_chars
        .saturating_mul(4)
        .clamp(4 * 1024, 8 * 1024 * 1024);
    let stdout_task = tokio::spawn(read_capped(stdout, byte_limit));
    let stderr_task = tokio::spawn(read_capped(stderr, byte_limit));

    let mut timed_out = false;
    let exit_code =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await {
            Ok(status) => status?.code().unwrap_or(-1),
            Err(_) => {
                timed_out = true;
                terminate_process_tree(&mut child).await;
                -1
            }
        };

    let stdout = stdout_task
        .await
        .map_err(|error| AppError::internal(format!("stdout reader task failed: {error}")))??;
    let stderr = stderr_task
        .await
        .map_err(|error| AppError::internal(format!("stderr reader task failed: {error}")))??;

    Ok(ProcessCapture {
        exit_code,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
        timed_out,
        truncated: stdout.truncated || stderr.truncated,
    })
}

pub async fn terminate_process_tree(child: &mut Child) {
    let Some(pid) = child.id() else {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return;
    };

    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(pid) {
            // SAFETY: a negative pid addresses the process group created before spawn.
            let _ = unsafe { libc::kill(-pid, libc::SIGKILL) };
        }
    }
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("taskkill.exe")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }

    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(unix)]
fn prepare_process_tree(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    command.as_std_mut().process_group(0);
}

#[cfg(not(unix))]
fn prepare_process_tree(_command: &mut Command) {}

async fn read_capped<R>(mut reader: R, limit: usize) -> io::Result<CappedBytes>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut output = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        if output.len() < limit {
            let remaining = limit - output.len();
            let retained = remaining.min(read);
            output.extend_from_slice(&buffer[..retained]);
            truncated |= retained < read;
        } else {
            truncated = true;
        }
    }
    Ok(CappedBytes {
        bytes: output,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn bounded_reader_drains_and_marks_truncation() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let write = tokio::spawn(async move {
            writer.write_all(b"0123456789abcdef").await.unwrap();
        });
        let result = read_capped(reader, 8).await.unwrap();
        write.await.unwrap();
        assert_eq!(result.bytes, b"01234567");
        assert!(result.truncated);
    }

    #[test]
    fn capability_limits_are_described() {
        assert!(!resource_limit_labels().is_empty());
    }
}
