extern crate tauri as tauri_crate;

#[macro_export]
macro_rules! openmind_generate_handler {
    ($($command:ident),* $(,)?) => {
        $crate::tauri_crate::generate_handler![
            $($command),*,
            transcribe_audio,
            ocr_pdf_pages,
            send_multimodal_chat_message,
            regenerate_multimodal_message,
            artifact_media_data_url,
            create_soundscape_artifact,
            connect_google_workspace,
            google_workspace_status,
            disconnect_google_workspace,
            execute_google_workspace_action,
            execute_github_workspace_action,
            integration_status,
            save_integration_config,
            clear_integration_config,
            connect_integration,
            disconnect_integration,
            execute_integration_action,
            connected_app_agent_status_for_conversation,
            send_connected_app_message,
            regenerate_connected_app_message,
            project_local_access_status,
            attach_project_workspace_folder,
            detach_project_workspace_folder,
            set_project_full_local_access,
            list_project_workspace_directory,
            read_project_workspace_file,
            write_project_workspace_file,
            create_project_workspace_directory,
            move_project_workspace_path,
            delete_project_workspace_path,
            run_project_terminal_command,
            project_agent_status_for_conversation,
            send_project_agent_message,
            regenerate_project_agent_message,
            platform_capabilities
        ]
    };
}

mod tauri {
    pub use crate::openmind_generate_handler as generate_handler;
    pub use crate::tauri_crate::*;
}

mod connected_agent;
mod connector_ecosystem;
mod connector_input_guard;
mod connector_stabilization;
mod github_workspace;
mod google_workspace;
mod local_agent;
mod local_workspace;
mod multimodal;
mod pdf_ocr;
mod platform;
mod speech_runtime;
mod vision_batch;
mod warm_start;

pub(crate) use connected_agent::{
    connected_app_agent_status_for_conversation, regenerate_connected_app_message,
    send_connected_app_message,
};
pub(crate) use connector_ecosystem::{
    clear_integration_config, connect_integration, disconnect_integration,
    execute_integration_action, integration_status, save_integration_config,
};
pub(crate) use connector_input_guard::{
    execute_github_workspace_action, execute_google_workspace_action,
};
pub(crate) use google_workspace::{
    connect_google_workspace, disconnect_google_workspace, google_workspace_status,
};
pub(crate) use local_agent::{
    project_agent_status_for_conversation, regenerate_project_agent_message,
    send_project_agent_message,
};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) use local_workspace::{
    attach_project_workspace_folder, create_project_workspace_directory,
    delete_project_workspace_path, detach_project_workspace_folder,
    list_project_workspace_directory, move_project_workspace_path, project_local_access_status,
    read_project_workspace_file, run_project_terminal_command, set_project_full_local_access,
    write_project_workspace_file,
};

#[cfg(any(target_os = "android", target_os = "ios"))]
mod mobile_local_workspace_stubs {
    use crate::{app_error::AppError, AppState};
    use serde_json::Value;
    use tauri::State;

    fn unavailable<T>() -> Result<T, AppError> {
        Err(AppError::internal(
            "Local machine workspace, Full PC access, and terminal execution are desktop-only. Mobile Projects use scoped app files and connected services instead.",
        ))
    }

    #[tauri::command]
    pub fn project_local_access_status(
        project_id: String,
        state: State<AppState>,
    ) -> Result<Value, AppError> {
        let _ = (project_id, state);
        unavailable()
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
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) use mobile_local_workspace_stubs::{
    attach_project_workspace_folder, create_project_workspace_directory,
    delete_project_workspace_path, detach_project_workspace_folder,
    list_project_workspace_directory, move_project_workspace_path, project_local_access_status,
    read_project_workspace_file, run_project_terminal_command, set_project_full_local_access,
    write_project_workspace_file,
};

pub(crate) use multimodal::{
    artifact_media_data_url, create_soundscape_artifact, regenerate_multimodal_message,
    send_multimodal_chat_message, transcribe_audio,
};
pub(crate) use pdf_ocr::ocr_pdf_pages;
pub(crate) use platform::platform_capabilities;

include!("lib_legacy.rs");

// Tauri mobile builds load this crate as a library through the native Android/iOS
// host instead of executing src/main.rs. Keep the existing desktop run() function
// untouched and provide the mobile entry wrapper at the crate root.
#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri_crate::mobile_entry_point]
pub fn mobile_entry() {
    run();
}
