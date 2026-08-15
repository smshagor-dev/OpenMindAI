use tracing_appender::non_blocking::WorkerGuard;

use crate::{app_error::AppError, portable_root::PortableRootManager};

/// Initializes structured app logging into `OPENMINDAI_ROOT/logs/`, rotated
/// daily. Deliberately never logs conversation content — chat messages only
/// ever flow through `ChatRepository`/SQLite (see `chat.rs`, `inference.rs`);
/// this logger is for startup/setup/maintenance/update *events*, not their
/// content.
///
/// Returns a guard that must stay alive for the process's lifetime (it
/// flushes the non-blocking writer on drop). Bind it in `run()` before the
/// call to `tauri::Builder::run`, which blocks until the app exits — do not
/// let it go out of scope any earlier or log lines will be silently lost.
pub fn init(root: &PortableRootManager) -> WorkerGuard {
    let file_appender = tracing_appender::rolling::daily(root.logs_dir(), "openmindai.log");
    let (writer, guard) = tracing_appender::non_blocking(file_appender);

    // Override with the OPENMINDAI_LOG env var (standard tracing-subscriber
    // EnvFilter syntax, e.g. "debug" or "open_mind_ai_lib=debug,warn") for
    // troubleshooting; defaults to "info" otherwise.
    let filter = tracing_subscriber::EnvFilter::try_from_env("OPENMINDAI_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .init();

    guard
}

/// The tail of today's (or the most recently written) structured app log,
/// for the Maintenance Center's in-app viewer — a quick look without
/// leaving the app, not a replacement for "Open Logs Folder" on the full
/// rotated history. Returns an empty string, not an error, if no app log
/// has been written yet (e.g. a dev root with `OPENMINDAI_LOG` filtering
/// everything out).
pub fn read_recent(root: &PortableRootManager, max_lines: usize) -> Result<String, AppError> {
    let logs_dir = root.logs_dir();
    let Ok(read_dir) = std::fs::read_dir(&logs_dir) else {
        return Ok(String::new());
    };

    let latest = read_dir
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("openmindai.log"))
        })
        .max_by_key(|entry| {
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
        });

    let Some(latest) = latest else {
        return Ok(String::new());
    };

    let content = std::fs::read_to_string(latest.path())?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    Ok(lines[start..].join("\n"))
}
