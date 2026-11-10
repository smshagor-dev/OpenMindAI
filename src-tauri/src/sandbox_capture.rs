//! Bounded readers for untrusted Windows Sandbox control files.
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

pub(crate) const MAX_CAPTURE_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_RESULT_BYTES: usize = 4096;

pub(crate) fn byte_limit(chars: usize) -> usize {
    chars.saturating_mul(4).min(MAX_CAPTURE_BYTES)
}

fn read_prefix(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sandbox output is not a regular file",
        ));
    }
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sandbox output changed type",
        ));
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn read_output(path: &Path, chars: usize) -> io::Result<(String, bool)> {
    let limit = byte_limit(chars);
    let mut bytes = match read_prefix(path, limit) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((String::new(), false)),
        value => value?,
    };
    let mut truncated = bytes.len() > limit;
    bytes.truncate(limit);
    let text = String::from_utf8_lossy(&bytes);
    let mut iter = text.chars();
    let result = iter.by_ref().take(chars.min(MAX_CAPTURE_BYTES)).collect();
    truncated |= iter.next().is_some();
    Ok((result, truncated))
}

pub(crate) fn read_result(path: &Path) -> io::Result<String> {
    let bytes = read_prefix(path, MAX_RESULT_BYTES)?;
    if bytes.len() > MAX_RESULT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows Sandbox result exceeds 4096 bytes",
        ));
    }
    String::from_utf8(bytes)
        .map(|s| s.trim_start_matches('\u{feff}').to_owned())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(windows)]
pub(crate) fn launcher(
    encoded: &str,
    cwd: &str,
    root: &str,
    chars: usize,
    timeout_ms: u64,
    shutdown: bool,
) -> String {
    let quote = |s: &str| s.replace('\'', "''");
    include_str!("windows_sandbox_capture.ps1")
        .replace("__ENCODED__", encoded)
        .replace("__CWD__", &quote(cwd))
        .replace("__STDOUT__", &quote(&format!("{root}\\stdout.txt")))
        .replace("__STDERR__", &quote(&format!("{root}\\stderr.txt")))
        .replace("__RESULT__", &quote(&format!("{root}\\result.json")))
        .replace("__LIMIT__", &chars.min(MAX_CAPTURE_BYTES / 4).to_string())
        .replace("__TIMEOUT__", &timeout_ms.min(i32::MAX as u64).to_string())
        .replace(
            "__SHUTDOWN__",
            if shutdown {
                "shutdown.exe /s /t 0 /f"
            } else {
                ""
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn huge_output_is_read_as_a_bounded_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output");
        let file = fs::File::create(&path).unwrap();
        file.set_len(1024 * 1024 * 1024).unwrap();
        let (text, truncated) = read_output(&path, 100).unwrap();
        assert_eq!(text.len(), 100);
        assert!(truncated);
        assert_eq!(byte_limit(usize::MAX), MAX_CAPTURE_BYTES);
    }
    #[test]
    fn utf8_and_oversized_results_are_handled_without_unbounded_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output");
        fs::write(&path, "বাংলা".repeat(20)).unwrap();
        let (text, truncated) = read_output(&path, 5).unwrap();
        assert_eq!(text.chars().count(), 5);
        assert!(truncated);
        fs::write(&path, vec![b' '; MAX_RESULT_BYTES + 1]).unwrap();
        assert!(read_result(&path).is_err());
        fs::write(&path, "\u{feff}{\"exitCode\":0}").unwrap();
        assert_eq!(read_result(&path).unwrap(), "{\"exitCode\":0}");
    }
    #[cfg(windows)]
    #[test]
    fn real_powershell_capture_bounds_both_pipes_and_handles_timeout() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        for timeout in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let code = if timeout {
                "Start-Sleep -Seconds 30"
            } else {
                "1..20000 | ForEach-Object { [Console]::Out.WriteLine('stdout spam'); [Console]::Error.WriteLine('stderr spam') }"
            };
            let encoded = STANDARD.encode(
                code.encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            let script = launcher(
                &encoded,
                &dir.path().display().to_string(),
                &dir.path().display().to_string(),
                100,
                if timeout { 100 } else { 20000 },
                false,
            );
            let path = dir.path().join("run.ps1");
            fs::write(&path, script).unwrap();
            let status = std::process::Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(path)
                .status()
                .unwrap();
            assert!(status.success());
            let result: serde_json::Value =
                serde_json::from_str(&read_result(&dir.path().join("result.json")).unwrap())
                    .unwrap();
            assert_eq!(result["timedOut"], timeout);
            if !timeout {
                assert_eq!(result["truncated"], true);
            }
            for name in ["stdout.txt", "stderr.txt"] {
                assert!(fs::metadata(dir.path().join(name)).unwrap().len() <= 400);
            }
        }
    }
}
