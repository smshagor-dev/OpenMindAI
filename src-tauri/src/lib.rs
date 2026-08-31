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
            mobile_local_inference_status,
            mobile_generate_text,
            mobile_model_recommendation,
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
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod local_agent;
#[cfg(any(target_os = "android", target_os = "ios"))]
#[path = "local_agent_mobile.rs"]
mod local_agent;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod local_workspace;
#[cfg(any(target_os = "android", target_os = "ios"))]
#[path = "local_workspace_mobile.rs"]
mod local_workspace;
mod mobile_inference;
mod mobile_model_policy;
mod multimodal;
mod pdf_ocr;
mod platform;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod speech_runtime;
#[cfg(any(target_os = "android", target_os = "ios"))]
#[path = "speech_runtime_mobile.rs"]
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
pub(crate) use local_workspace::{
    attach_project_workspace_folder, create_project_workspace_directory,
    delete_project_workspace_path, detach_project_workspace_folder,
    list_project_workspace_directory, move_project_workspace_path, project_local_access_status,
    read_project_workspace_file, run_project_terminal_command, set_project_full_local_access,
    write_project_workspace_file,
};
pub(crate) use mobile_inference::{mobile_generate_text, mobile_local_inference_status};
pub(crate) use mobile_model_policy::mobile_model_recommendation;
pub(crate) use multimodal::{
    artifact_media_data_url, create_soundscape_artifact, regenerate_multimodal_message,
    send_multimodal_chat_message, transcribe_audio,
};
pub(crate) use pdf_ocr::ocr_pdf_pages;
pub(crate) use platform::platform_capabilities;

include!("lib_legacy.rs");

#[cfg(any(target_os = "android", target_os = "ios"))]
fn run_mobile() {
    use tauri_crate::Manager as _;

    tauri_crate::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Mobile never resolves the desktop portable/external-drive root. Keep all
            // databases, downloaded assets, artifacts, and logs inside the OS-managed,
            // bundle-scoped app-local data directory.
            let app_data = app.path().app_local_data_dir()?;
            let root = PortableRootManager::from_root(app_data.join("OpenMindAI"));
            root.ensure_directories()?;
            let database_path = root.database_path();
            let database = Database::open(database_path.clone())?;
            let hardware = HardwareProfiler::detect();

            app.manage(AppState {
                runtime: Mutex::new(LlamaRuntimeManager::new(root.clone())),
                downloads: ModelDownloadManager::new(root.clone()),
                runtime_installer: RuntimeInstaller::new(root.clone()),
                root,
                hardware,
                active_database_path: database_path,
                database: Mutex::new(database),
                active_generations: ActiveGenerations::default(),
                warm_start: warm_start::WarmStartCoordinator::default(),
                http: Client::new(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_portable_root,
            installation_status,
            complete_setup,
            save_setup_progress,
            mark_runtime_ready,
            mark_model_ready,
            check_storage_location,
            list_conversations,
            create_conversation,
            rename_conversation,
            set_conversation_pinned,
            set_conversation_model,
            archive_conversation,
            delete_conversation,
            list_messages,
            add_user_message,
            create_streaming_assistant_message,
            append_message_chunk,
            complete_message,
            delete_message,
            list_projects,
            create_project,
            update_project,
            delete_project,
            link_project_conversation,
            unlink_project_conversation,
            add_project_file,
            delete_project_file,
            create_text_artifact,
            create_document_artifact,
            create_generation_artifact,
            list_artifacts,
            list_library_entries,
            open_artifact,
            open_external_url,
            reveal_artifact_in_folder,
            detect_hardware,
            get_performance_profile,
            discover_models,
            get_qwen_download_status,
            get_model_download_status,
            download_qwen_model,
            download_catalog_model,
            cancel_qwen_download,
            cancel_model_download,
            pause_model_download,
            delete_catalog_model,
            validate_model,
            plan_model_launch,
            activate_model,
            get_storage_summary,
            clear_cache,
            run_diagnostics,
            repair_installation,
            backup_database,
            list_backups,
            check_model_updates,
            open_maintenance_folder,
            read_recent_logs,
            get_llama_runtime_status,
            get_llama_runtime_inventory,
            get_runtime_install_status,
            install_recommended_runtime,
            cancel_runtime_install,
            start_llama_runtime,
            stop_llama_runtime,
            send_chat_message,
            regenerate_message,
            cancel_generation,
            get_app_preferences,
            save_app_preferences,
            get_user_profile,
            save_user_profile,
            get_github_account,
            save_github_token,
            disconnect_github,
            list_github_repos,
            list_github_issues,
            get_google_credentials,
            save_google_credentials,
            clear_google_credentials
        ])
        .run(tauri_crate::generate_context!())
        .expect("error while running OpenMindAI mobile");
}

// Tauri mobile builds load this crate as a library through the native Android/iOS
// host instead of executing src/main.rs. Mobile startup intentionally uses a
// separate app-data bootstrap so the desktop portable-root/runtime policy is never run.
#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri_crate::mobile_entry_point]
pub fn mobile_entry() {
    run_mobile();
}
