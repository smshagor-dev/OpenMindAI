use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{app_error::AppError, chat::Message, AppState};

const MOBILE_AGENT_MESSAGE: &str =
    "Desktop Project Agent terminal and filesystem execution is unavailable on mobile. Use mobile-scoped Projects or connected services instead.";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAgentStatus {
    pub available: bool,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub full_pc_access: bool,
    pub terminal_enabled: bool,
    pub attached_roots: usize,
}

#[tauri::command]
pub fn project_agent_status_for_conversation(
    conversation_id: String,
    state: State<AppState>,
) -> Result<ProjectAgentStatus, AppError> {
    let _ = (conversation_id, state);
    Ok(ProjectAgentStatus {
        available: false,
        project_id: None,
        project_name: None,
        full_pc_access: false,
        terminal_enabled: false,
        attached_roots: 0,
    })
}

#[tauri::command]
pub async fn send_project_agent_message(
    app: AppHandle,
    conversation_id: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<Message, AppError> {
    let _ = (app, conversation_id, content, state);
    Err(AppError::internal(MOBILE_AGENT_MESSAGE))
}

#[tauri::command]
pub async fn regenerate_project_agent_message(
    app: AppHandle,
    conversation_id: String,
    assistant_message_id: String,
    state: State<'_, AppState>,
) -> Result<Message, AppError> {
    let _ = (app, conversation_id, assistant_message_id, state);
    Err(AppError::internal(MOBILE_AGENT_MESSAGE))
}
