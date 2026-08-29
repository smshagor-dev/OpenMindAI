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
            run_project_terminal_command
        ]
    };
}

mod tauri {
    pub use crate::openmind_generate_handler as generate_handler;
    pub use crate::tauri_crate::*;
}

mod connector_ecosystem;
mod connector_input_guard;
mod connector_stabilization;
mod github_workspace;
mod google_workspace;
mod local_workspace;
mod multimodal;
mod pdf_ocr;
mod speech_runtime;
mod vision_batch;

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
pub(crate) use local_workspace::{
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

include!("lib_legacy.rs");
