use serde_json::{json, Value};
use tauri::State;

use crate::{app_error::AppError, AppState};

const MOBILE_WORKSPACE_MESSAGE: &str =
    "Local machine workspace, Full PC access, and terminal execution are desktop-only. Mobile Projects use scoped app files and connected services instead.";

fn unavailable<T>() -> Result<T, AppError> {
    Err(AppError::internal(MOBILE_WORKSPACE_MESSAGE))
}

#[tauri::command]
pub fn project_local_access_status(
    project_id: String,
    state: State<AppState>,
) -> Result<Value, AppError> {
    let _ = state;
    Ok(json!({
        "projectId": project_id,
        "fullPcAccess": false,
        "terminalEnabled": false,
        "roots": [],
        "mobileScoped": true
    }))
}

#[tauri::command]
pub fn attach_project_workspace_folder(
    project_id: String,
    path: String,
    state: State<AppState>,
) -> Result<Value, AppError> {
    let _ = (project_id, path, state);
    unavailable()
}

#[tauri::command]
pub fn detach_project_workspace_folder(
    project_id: String,
    root_id: String,
    state: State<AppState>,
) -> Result<Value, AppError> {
    let _ = (project_id, root_id, state);
    unavailable()
}

#[tauri::command]
pub fn set_project_full_local_access(
    project_id: String,
    enabled: bool,
    approved: bool,
    state: State<AppState>,
) -> Result<Value, AppError> {
    let _ = (project_id, enabled, approved, state);
    unavailable()
}

#[tauri::command]
pub fn list_project_workspace_directory(
    project_id: String,
    root_id: Option<String>,
    path: String,
    state: State<AppState>,
) -> Result<Value, AppError> {
    let _ = (project_id, root_id, path, state);
    unavailable()
}

#[tauri::command]
pub fn read_project_workspace_file(
    project_id: String,
    root_id: Option<String>,
    path: String,
    state: State<AppState>,
) -> Result<Value, AppError> {
    let _ = (project_id, root_id, path, state);
    unavailable()
}

#[tauri::command]
pub fn write_project_workspace_file(
    project_id: String,
    root_id: Option<String>,
    path: String,
    content: String,
    approved: bool,
    state: State<AppState>,
) -> Result<Value, AppError> {
    let _ = (project_id, root_id, path, content, approved, state);
    unavailable()
}

#[tauri::command]
pub fn create_project_workspace_directory(
    project_id: String,
    root_id: Option<String>,
    path: String,
    approved: bool,
    state: State<AppState>,
) -> Result<Value, AppError> {
    let _ = (project_id, root_id, path, approved, state);
    unavailable()
}

#[tauri::command]
pub fn move_project_workspace_path(
    project_id: String,
    root_id: Option<String>,
    source_path: String,
    target_path: String,
    approved: bool,
    state: State<AppState>,
) -> Result<Value, AppError> {
    let _ = (
        project_id,
        root_id,
        source_path,
        target_path,
        approved,
        state,
    );
    unavailable()
}

#[tauri::command]
pub fn delete_project_workspace_path(
    project_id: String,
    root_id: Option<String>,
    path: String,
    approved: bool,
    state: State<AppState>,
) -> Result<(), AppError> {
    let _ = (project_id, root_id, path, approved, state);
    unavailable()
}

#[tauri::command]
pub async fn run_project_terminal_command(
    project_id: String,
    root_id: Option<String>,
    cwd: String,
    command: String,
    approved: bool,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    let _ = (project_id, root_id, cwd, command, approved, state);
    unavailable()
}
