use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::process::Command;
use uuid::Uuid;

use crate::{
    app_error::AppError,
    database::Database,
    portable_root::{preview_writable, strip_windows_verbatim_prefix},
    AppState,
};

const SETTINGS_PREFIX: &str = "project.local_workspace.";
const MAX_ROOTS: usize = 12;
const MAX_DIRECTORY_ENTRIES: usize = 500;
const MAX_TEXT_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_WRITE_CHARS: usize = 2_000_000;
const MAX_TERMINAL_COMMAND_CHARS: usize = 12_000;
const MAX_TERMINAL_OUTPUT_CHARS: usize = 200_000;
const TERMINAL_TIMEOUT_SECS: u64 = 120;
const WORKSPACE_CONTEXT_PATHS: usize = 120;
const WORKSPACE_CONTEXT_DEPTH: usize = 4;
const WORKSPACE_CONTEXT_FILES: usize = 6;
const WORKSPACE_CONTEXT_FILE_CHARS: usize = 3_000;
const WORKSPACE_CONTEXT_TOTAL_CHARS: usize = 18_000;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ProjectWorkspaceConfig {
    #[serde(default)]
    full_pc_access: bool,
    #[serde(default)]
    roots: Vec<WorkspaceRootConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceRootConfig {
    id: String,
    path: String,
    created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLocalAccessStatus {
    pub project_id: String,
    pub full_pc_access: bool,
    pub terminal_enabled: bool,
    pub roots: Vec<ProjectWorkspaceRoot>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWorkspaceRoot {
    pub id: String,
    pub path: String,
    pub label: String,
    pub exists: bool,
    pub writable: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    pub name: String,
    pub path: String,
    pub relative_path: String,
    pub kind: String,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub hidden: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileContent {
    pub path: String,
    pub content: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMutationResult {
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCommandResult {
    pub command: String,
    pub cwd: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
    pub timed_out: bool,
    pub truncated: bool,
}

#[tauri::command]
pub fn project_local_access_status(
    project_id: String,
    state: State<AppState>,
) -> Result<ProjectLocalAccessStatus, AppError> {
    let db = lock_database(&state)?;
    ensure_project_exists(&db, &project_id)?;
    let config = load_config(&db, &project_id)?;
    Ok(status_from_config(&project_id, &config))
}

#[tauri::command]
pub fn attach_project_workspace_folder(
    project_id: String,
    path: String,
    state: State<AppState>,
) -> Result<ProjectLocalAccessStatus, AppError> {
    let candidate = normalize_existing_directory(Path::new(path.trim()))?;
    let display_path = display_path(&candidate);
    let db = lock_database(&state)?;
    ensure_project_exists(&db, &project_id)?;
    let mut config = load_config(&db, &project_id)?;

    if !config
        .roots
        .iter()
        .any(|root| same_path(&root.path, &display_path))
    {
        if config.roots.len() >= MAX_ROOTS {
            return Err(AppError::internal(format!(
                "a project can attach at most {MAX_ROOTS} local folders"
            )));
        }
        config.roots.push(WorkspaceRootConfig {
            id: Uuid::new_v4().to_string(),
            path: display_path,
            created_at: Utc::now().to_rfc3339(),
        });
        save_config(&db, &project_id, &config)?;
    }

    Ok(status_from_config(&project_id, &config))
}

#[tauri::command]
pub fn detach_project_workspace_folder(
    project_id: String,
    root_id: String,
    state: State<AppState>,
) -> Result<ProjectLocalAccessStatus, AppError> {
    let db = lock_database(&state)?;
    ensure_project_exists(&db, &project_id)?;
    let mut config = load_config(&db, &project_id)?;
    let previous = config.roots.len();
    config.roots.retain(|root| root.id != root_id);
    if config.roots.len() == previous {
        return Err(AppError::internal("attached project folder not found"));
    }
    save_config(&db, &project_id, &config)?;
    Ok(status_from_config(&project_id, &config))
}

#[tauri::command]
pub fn set_project_full_local_access(
    project_id: String,
    enabled: bool,
    approved: bool,
    state: State<AppState>,
) -> Result<ProjectLocalAccessStatus, AppError> {
    if enabled && !approved {
        return Err(AppError::internal(
            "enabling full PC and terminal access requires explicit approval",
        ));
    }
    let db = lock_database(&state)?;
    ensure_project_exists(&db, &project_id)?;
    let mut config = load_config(&db, &project_id)?;
    config.full_pc_access = enabled;
    save_config(&db, &project_id, &config)?;
    Ok(status_from_config(&project_id, &config))
}

#[tauri::command]
pub fn list_project_workspace_directory(
    project_id: String,
    root_id: Option<String>,
    path: String,
    state: State<AppState>,
) -> Result<Vec<WorkspaceEntry>, AppError> {
    let config = load_project_config(&state, &project_id)?;
    let directory = resolve_path(&config, root_id.as_deref(), &path, true)?;
    if !directory.is_dir() {
        return Err(AppError::internal("workspace path is not a directory"));
    }

    let base = selected_root_path(&config, root_id.as_deref()).ok();
    let mut entries = Vec::new();
    for item in fs::read_dir(&directory)?.take(MAX_DIRECTORY_ENTRIES) {
        let item = item?;
        let item_path = item.path();
        let metadata = fs::symlink_metadata(&item_path)?;
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            "symlink"
        } else if file_type.is_dir() {
            "directory"
        } else if file_type.is_file() {
            "file"
        } else {
            "other"
        };
        let name = item.file_name().to_string_lossy().into_owned();
        let relative_path = base
            .as_ref()
            .and_then(|root| item_path.strip_prefix(root).ok())
            .map(|relative| relative.to_string_lossy().into_owned())
            .unwrap_or_else(|| display_path(&item_path));
        entries.push(WorkspaceEntry {
            hidden: name.starts_with('.'),
            name,
            path: display_path(&item_path),
            relative_path,
            kind: kind.to_string(),
            size_bytes: file_type.is_file().then_some(metadata.len()),
            modified_at: metadata.modified().ok().map(system_time_string),
        });
    }
    entries.sort_by(|left, right| {
        let left_dir = left.kind == "directory";
        let right_dir = right.kind == "directory";
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

#[tauri::command]
pub fn read_project_workspace_file(
    project_id: String,
    root_id: Option<String>,
    path: String,
    state: State<AppState>,
) -> Result<WorkspaceFileContent, AppError> {
    let config = load_project_config(&state, &project_id)?;
    let file_path = resolve_path(&config, root_id.as_deref(), &path, true)?;
    let metadata = fs::metadata(&file_path)?;
    if !metadata.is_file() {
        return Err(AppError::internal("workspace path is not a file"));
    }
    if metadata.len() > MAX_TEXT_FILE_BYTES {
        return Err(AppError::internal(format!(
            "file exceeds the {} MiB editor limit",
            MAX_TEXT_FILE_BYTES / 1024 / 1024
        )));
    }
    let bytes = fs::read(&file_path)?;
    let content = String::from_utf8(bytes).map_err(|_| {
        AppError::internal("binary or non-UTF-8 files cannot be opened in the text editor")
    })?;
    Ok(WorkspaceFileContent {
        path: display_path(&file_path),
        content,
        size_bytes: metadata.len(),
    })
}

#[tauri::command]
pub fn write_project_workspace_file(
    project_id: String,
    root_id: Option<String>,
    path: String,
    content: String,
    approved: bool,
    state: State<AppState>,
) -> Result<WorkspaceMutationResult, AppError> {
    if !approved {
        return Err(AppError::internal(
            "saving a local file requires explicit approval",
        ));
    }
    if content.chars().count() > MAX_WRITE_CHARS {
        return Err(AppError::internal(
            "file content exceeds the local editor limit",
        ));
    }
    let config = load_project_config(&state, &project_id)?;
    let file_path = resolve_path(&config, root_id.as_deref(), &path, false)?;
    if file_path.exists() && file_path.is_dir() {
        return Err(AppError::internal("cannot overwrite a directory as a file"));
    }
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&file_path, content.as_bytes())?;
    Ok(WorkspaceMutationResult {
        path: display_path(&file_path),
        kind: "file".to_string(),
    })
}

#[tauri::command]
pub fn create_project_workspace_directory(
    project_id: String,
    root_id: Option<String>,
    path: String,
    approved: bool,
    state: State<AppState>,
) -> Result<WorkspaceMutationResult, AppError> {
    if !approved {
        return Err(AppError::internal(
            "creating a local folder requires explicit approval",
        ));
    }
    let config = load_project_config(&state, &project_id)?;
    let directory = resolve_path(&config, root_id.as_deref(), &path, false)?;
    fs::create_dir_all(&directory)?;
    Ok(WorkspaceMutationResult {
        path: display_path(&directory),
        kind: "directory".to_string(),
    })
}

#[tauri::command]
pub fn move_project_workspace_path(
    project_id: String,
    root_id: Option<String>,
    source_path: String,
    target_path: String,
    approved: bool,
    state: State<AppState>,
) -> Result<WorkspaceMutationResult, AppError> {
    if !approved {
        return Err(AppError::internal(
            "moving or renaming a local path requires explicit approval",
        ));
    }
    let config = load_project_config(&state, &project_id)?;
    let source = resolve_path(&config, root_id.as_deref(), &source_path, true)?;
    reject_attached_root_mutation(&config, &source)?;
    let target = resolve_path(&config, root_id.as_deref(), &target_path, false)?;
    if target.exists() {
        return Err(AppError::internal("target path already exists"));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let kind = if source.is_dir() { "directory" } else { "file" };
    fs::rename(&source, &target)?;
    Ok(WorkspaceMutationResult {
        path: display_path(&target),
        kind: kind.to_string(),
    })
}

#[tauri::command]
pub fn delete_project_workspace_path(
    project_id: String,
    root_id: Option<String>,
    path: String,
    approved: bool,
    state: State<AppState>,
) -> Result<(), AppError> {
    if !approved {
        return Err(AppError::internal(
            "deleting a local path requires explicit approval",
        ));
    }
    let config = load_project_config(&state, &project_id)?;
    let target = resolve_path(&config, root_id.as_deref(), &path, true)?;
    reject_attached_root_mutation(&config, &target)?;
    reject_filesystem_root(&target)?;
    if target.is_dir() {
        fs::remove_dir_all(target)?;
    } else {
        fs::remove_file(target)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn run_project_terminal_command(
    project_id: String,
    root_id: Option<String>,
    cwd: String,
    command: String,
    approved: bool,
    state: State<'_, AppState>,
) -> Result<TerminalCommandResult, AppError> {
    if !approved {
        return Err(AppError::internal(
            "running a terminal command requires explicit approval",
        ));
    }
    let command_text = command.trim();
    if command_text.is_empty() {
        return Err(AppError::internal("terminal command cannot be empty"));
    }
    if command_text.chars().count() > MAX_TERMINAL_COMMAND_CHARS {
        return Err(AppError::internal(
            "terminal command exceeds the safety limit",
        ));
    }

    let config = load_project_config(&state, &project_id)?;
    if !config.full_pc_access {
        return Err(AppError::internal(
            "terminal access is disabled until Full PC + Terminal access is explicitly enabled for this project",
        ));
    }

    let start_dir = if cwd.trim().is_empty() {
        selected_root_path(&config, root_id.as_deref())?
    } else {
        resolve_path(&config, root_id.as_deref(), &cwd, true)?
    };
    if !start_dir.is_dir() {
        return Err(AppError::internal(
            "terminal working directory is not a directory",
        ));
    }

    let started = Instant::now();
    let mut process = terminal_process(command_text, &start_dir);
    process.kill_on_drop(true);
    let output =
        match tokio::time::timeout(Duration::from_secs(TERMINAL_TIMEOUT_SECS), process.output())
            .await
        {
            Ok(result) => result?,
            Err(_) => {
                return Ok(TerminalCommandResult {
                    command: command_text.to_string(),
                    cwd: display_path(&start_dir),
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: format!("Command timed out after {TERMINAL_TIMEOUT_SECS} seconds."),
                    duration_ms: started.elapsed().as_millis(),
                    timed_out: true,
                    truncated: false,
                });
            }
        };

    let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let resolved_cwd = take_terminal_cwd(&mut stdout).unwrap_or_else(|| display_path(&start_dir));
    let (stdout, stdout_truncated) = truncate_chars(&stdout, MAX_TERMINAL_OUTPUT_CHARS);
    let (stderr, stderr_truncated) = truncate_chars(&stderr, MAX_TERMINAL_OUTPUT_CHARS);

    Ok(TerminalCommandResult {
        command: command_text.to_string(),
        cwd: resolved_cwd,
        exit_code: output.status.code().unwrap_or(-1),
        stdout,
        stderr,
        duration_ms: started.elapsed().as_millis(),
        timed_out: false,
        truncated: stdout_truncated || stderr_truncated,
    })
}

pub(crate) fn workspace_context_for_project(
    database: &Database,
    project_id: &str,
) -> Result<Option<String>, AppError> {
    let config = load_config(database, project_id)?;
    if config.roots.is_empty() {
        return Ok(None);
    }

    let mut sections =
        vec![format!(
        "[open-mind-ai-local-workspace]\nAttached local folders: {}\nFull PC + terminal access: {}",
        config.roots.len(),
        if config.full_pc_access { "enabled" } else { "disabled" }
    )];
    let mut remaining = WORKSPACE_CONTEXT_TOTAL_CHARS;

    for root in config.roots.iter().take(3) {
        if remaining == 0 {
            break;
        }
        let root_path = PathBuf::from(&root.path);
        if !root_path.is_dir() {
            continue;
        }
        let mut paths = Vec::new();
        collect_workspace_paths(&root_path, &root_path, 0, &mut paths);
        let mut section = format!("Folder: {}\n", display_path(&root_path));
        if !paths.is_empty() {
            section.push_str("Workspace tree:\n");
            for path in paths.iter().take(WORKSPACE_CONTEXT_PATHS) {
                section.push_str("- ");
                section.push_str(path);
                section.push('\n');
            }
        }

        let key_files = key_context_files(&root_path);
        for file in key_files.into_iter().take(WORKSPACE_CONTEXT_FILES) {
            if remaining == 0 {
                break;
            }
            if let Ok(content) = fs::read_to_string(&file) {
                let relative = file
                    .strip_prefix(&root_path)
                    .unwrap_or(&file)
                    .to_string_lossy();
                let (snippet, _) = truncate_chars(&content, WORKSPACE_CONTEXT_FILE_CHARS);
                section.push_str(&format!("\n--- {relative} ---\n{snippet}\n"));
            }
        }

        let (bounded, _) = truncate_chars(&section, remaining);
        remaining = remaining.saturating_sub(bounded.chars().count());
        sections.push(bounded);
    }

    Ok(Some(sections.join("\n\n")))
}

fn lock_database(state: &State<AppState>) -> Result<std::sync::MutexGuard<'_, Database>, AppError> {
    state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))
}

fn load_project_config(
    state: &State<AppState>,
    project_id: &str,
) -> Result<ProjectWorkspaceConfig, AppError> {
    let db = lock_database(state)?;
    ensure_project_exists(&db, project_id)?;
    load_config(&db, project_id)
}

fn ensure_project_exists(database: &Database, project_id: &str) -> Result<(), AppError> {
    let exists: bool = database.connection().query_row(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
        params![project_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(AppError::internal(format!(
            "project not found: {project_id}"
        )));
    }
    Ok(())
}

fn config_key(project_id: &str) -> String {
    format!("{SETTINGS_PREFIX}{project_id}")
}

fn load_config(database: &Database, project_id: &str) -> Result<ProjectWorkspaceConfig, AppError> {
    let raw: Option<String> = database
        .connection()
        .query_row(
            "SELECT value_json FROM app_settings WHERE key = ?1",
            params![config_key(project_id)],
            |row| row.get(0),
        )
        .optional()?;
    match raw {
        Some(raw) => serde_json::from_str(&raw).map_err(|error| {
            AppError::internal(format!(
                "invalid project local workspace configuration: {error}"
            ))
        }),
        None => Ok(ProjectWorkspaceConfig::default()),
    }
}

fn save_config(
    database: &Database,
    project_id: &str,
    config: &ProjectWorkspaceConfig,
) -> Result<(), AppError> {
    let value = serde_json::to_string(config).map_err(|error| {
        AppError::internal(format!("failed to serialize project local access: {error}"))
    })?;
    database.connection().execute(
        "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
        params![config_key(project_id), value, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

fn status_from_config(
    project_id: &str,
    config: &ProjectWorkspaceConfig,
) -> ProjectLocalAccessStatus {
    ProjectLocalAccessStatus {
        project_id: project_id.to_string(),
        full_pc_access: config.full_pc_access,
        terminal_enabled: config.full_pc_access,
        roots: config.roots.iter().map(root_status).collect(),
    }
}

fn root_status(root: &WorkspaceRootConfig) -> ProjectWorkspaceRoot {
    let path = PathBuf::from(&root.path);
    ProjectWorkspaceRoot {
        id: root.id.clone(),
        path: root.path.clone(),
        label: path
            .file_name()
            .filter(|name| !name.is_empty())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.path.clone()),
        exists: path.is_dir(),
        writable: path.is_dir() && preview_writable(&path),
        created_at: root.created_at.clone(),
    }
}

fn normalize_existing_directory(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().is_empty() {
        return Err(AppError::internal("folder path cannot be empty"));
    }
    let canonical = fs::canonicalize(path)?;
    if !canonical.is_dir() {
        return Err(AppError::internal(
            "selected workspace path is not a directory",
        ));
    }
    Ok(canonical)
}

fn selected_root<'a>(
    config: &'a ProjectWorkspaceConfig,
    root_id: Option<&str>,
) -> Result<&'a WorkspaceRootConfig, AppError> {
    let id = root_id.ok_or_else(|| AppError::internal("an attached project folder is required"))?;
    config
        .roots
        .iter()
        .find(|root| root.id == id)
        .ok_or_else(|| AppError::internal("attached project folder not found"))
}

fn selected_root_path(
    config: &ProjectWorkspaceConfig,
    root_id: Option<&str>,
) -> Result<PathBuf, AppError> {
    let root = selected_root(config, root_id)?;
    normalize_existing_directory(Path::new(&root.path))
}

fn resolve_path(
    config: &ProjectWorkspaceConfig,
    root_id: Option<&str>,
    input: &str,
    must_exist: bool,
) -> Result<PathBuf, AppError> {
    let raw = input.trim();
    if raw.contains('\0') {
        return Err(AppError::internal("path contains a null character"));
    }

    let supplied = if raw.is_empty() {
        Path::new(".")
    } else {
        Path::new(raw)
    };
    let (candidate, scoped_root) = if supplied.is_absolute() {
        if !config.full_pc_access {
            return Err(AppError::internal(
                "absolute paths require Full PC + Terminal access for this project",
            ));
        }
        (supplied.to_path_buf(), None)
    } else {
        let root = selected_root_path(config, root_id)?;
        (root.join(supplied), Some(root))
    };

    let resolved = if must_exist {
        fs::canonicalize(&candidate)?
    } else {
        resolve_new_path(&candidate)?
    };

    if !config.full_pc_access {
        let root = scoped_root.ok_or_else(|| AppError::internal("project folder scope missing"))?;
        let canonical_root = fs::canonicalize(&root)?;
        let security_path = if resolved.exists() {
            fs::canonicalize(&resolved)?
        } else {
            canonical_existing_parent(&resolved)?
        };
        if !security_path.starts_with(&canonical_root) {
            return Err(AppError::internal(
                "workspace path escaped the attached project folder",
            ));
        }
    }

    Ok(resolved)
}

fn resolve_new_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.exists() {
        return fs::canonicalize(path).map_err(AppError::from);
    }

    let mut probe = path.to_path_buf();
    let mut missing = Vec::new();
    while !probe.exists() {
        let name = probe
            .file_name()
            .ok_or_else(|| AppError::internal("path requires a file or folder name"))?
            .to_os_string();
        missing.push(name);
        if !probe.pop() {
            return Err(AppError::internal(
                "no existing parent directory found for path",
            ));
        }
    }

    let mut resolved = fs::canonicalize(probe)?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn canonical_existing_parent(path: &Path) -> Result<PathBuf, AppError> {
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        if !probe.pop() {
            return Err(AppError::internal(
                "no existing parent directory found for path",
            ));
        }
    }
    fs::canonicalize(probe).map_err(AppError::from)
}

fn reject_attached_root_mutation(
    config: &ProjectWorkspaceConfig,
    path: &Path,
) -> Result<(), AppError> {
    let canonical = fs::canonicalize(path)?;
    for root in &config.roots {
        if let Ok(root_path) = fs::canonicalize(&root.path) {
            if canonical == root_path {
                return Err(AppError::internal(
                    "detach an attached workspace folder instead of renaming or deleting its root",
                ));
            }
        }
    }
    Ok(())
}

fn reject_filesystem_root(path: &Path) -> Result<(), AppError> {
    if path.parent().is_none() {
        return Err(AppError::internal("refusing to delete a filesystem root"));
    }
    Ok(())
}

fn same_path(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn display_path(path: &Path) -> String {
    strip_windows_verbatim_prefix(path).display().to_string()
}

fn system_time_string(value: SystemTime) -> String {
    DateTime::<Utc>::from(value).to_rfc3339()
}

fn truncate_chars(value: &str, limit: usize) -> (String, bool) {
    let mut chars = value.chars();
    let output: String = chars.by_ref().take(limit).collect();
    (output, chars.next().is_some())
}

fn terminal_process(command: &str, cwd: &Path) -> Command {
    #[cfg(target_os = "windows")]
    {
        let wrapped = format!(
            "& {{ {command}; $openmindCode = if ($null -ne $LASTEXITCODE) {{ $LASTEXITCODE }} else {{ 0 }}; Write-Output \"__OPENMIND_CWD__$((Get-Location).Path)\"; exit $openmindCode }}"
        );
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
        let wrapped = format!(
            "{{ {command}; }}; openmind_code=$?; printf '\\n__OPENMIND_CWD__%s\\n' \"$PWD\"; exit $openmind_code"
        );
        let mut process = Command::new("/bin/sh");
        process.arg("-lc").arg(wrapped).current_dir(cwd);
        process
    }
}

fn take_terminal_cwd(stdout: &mut String) -> Option<String> {
    const MARKER: &str = "__OPENMIND_CWD__";
    let index = stdout.rfind(MARKER)?;
    let cwd = stdout[index + MARKER.len()..]
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

fn collect_workspace_paths(root: &Path, current: &Path, depth: usize, output: &mut Vec<String>) {
    if depth > WORKSPACE_CONTEXT_DEPTH || output.len() >= WORKSPACE_CONTEXT_PATHS {
        return;
    }
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
    for entry in entries {
        if output.len() >= WORKSPACE_CONTEXT_PATHS {
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if should_skip_context_name(&name) {
            continue;
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            output.push(format!("{relative}/"));
            if !file_type.is_symlink() {
                collect_workspace_paths(root, &path, depth + 1, output);
            }
        } else if file_type.is_file() {
            output.push(relative);
        }
    }
}

fn should_skip_context_name(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | ".next"
            | ".venv"
            | "venv"
            | "cache"
            | ".cache"
    )
}

fn key_context_files(root: &Path) -> Vec<PathBuf> {
    const NAMES: &[&str] = &[
        "README.md",
        "README",
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "requirements.txt",
        "go.mod",
        "composer.json",
        ".env.example",
    ];
    NAMES
        .iter()
        .map(|name| root.join(name))
        .filter(|path| path.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_is_unicode_safe() {
        let (value, truncated) = truncate_chars("a😀b", 2);
        assert_eq!(value, "a😀");
        assert!(truncated);
    }

    #[test]
    fn filesystem_root_delete_is_rejected() {
        let root = if cfg!(windows) {
            PathBuf::from(r"C:\\")
        } else {
            PathBuf::from("/")
        };
        assert!(reject_filesystem_root(&root).is_err());
    }

    #[test]
    fn nested_new_path_preserves_missing_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        fs::create_dir_all(&root).unwrap();
        let resolved = resolve_new_path(&root.join("new/sub/file.txt")).unwrap();
        assert_eq!(
            resolved,
            root.canonicalize().unwrap().join("new/sub/file.txt")
        );
    }
    #[test]
    fn scoped_new_path_cannot_escape_through_parent_segments() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        fs::create_dir_all(&root).unwrap();
        let config = ProjectWorkspaceConfig {
            full_pc_access: false,
            roots: vec![WorkspaceRootConfig {
                id: "root".to_string(),
                path: root.display().to_string(),
                created_at: "now".to_string(),
            }],
        };
        let result = resolve_path(&config, Some("root"), "../outside.txt", false);
        assert!(result.is_err());
    }
}
