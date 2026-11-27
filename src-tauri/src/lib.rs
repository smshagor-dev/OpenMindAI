mod cloud_bridge;
mod connected_action_ledger;
mod private_descriptor;
#[cfg(any(windows, test))]
mod sandbox_capture;
mod studio3d;
mod studio_outbox;
extern crate tauri as tauri_crate;

#[macro_export]
macro_rules! openmind_generate_handler {
    ($($command:ident),* $(,)?) => {
        $crate::tauri_crate::generate_handler![
            $($command),*,
            cloud_connect,
            cloud_login,
            cloud_disconnect,
            cloud_status,
            cloud_request,
            cloud_download_mesh,
            cloud_begin_regeneration,
            studio_outbox_recover,
            studio_outbox_list,
            studio_outbox_enqueue,
            studio_outbox_claim,
            studio_outbox_finish,
            studio_outbox_retry,
            create_studio3d_project,
            list_studio3d_projects,
            list_studio3d_scenes,
            save_studio3d_scene,
            load_studio3d_scene,
            delete_studio3d_scene,
            duplicate_studio3d_scene,
            restore_studio3d_scene_revision,
            import_studio3d_image,
            import_studio3d_glb,
            store_studio3d_generated_glb,
            import_studio3d_mesh,
            import_studio3d_gltf,
            create_studio3d_job,
            list_studio3d_jobs,
            transition_studio3d_job,
            execute_studio3d_local_job,
            cancel_studio3d_job,
            studio3d_ai_edit_object,
            studio3d_cloud_edit_object,
            list_studio3d_assets,
            load_studio3d_asset,
            delete_studio3d_asset,
            local_gpu_request,
            local_gpu_start,
            local_gpu_stop,
            studio3d_dependency_plan,
            studio3d_install_dependencies,
            studio3d_cancel_dependency_install,
            studio3d_neural_capabilities,
            prepare_default_chat_runtime,
            reveal_main_window,
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
            list_openagent_runs,
            openagent_run_details,
            restore_openagent_checkpoint,
            send_project_agent_message,
            regenerate_project_agent_message,
            resume_coding_run,
            coding_run_snapshot,
            approve_coding_action,
            reject_coding_action,
            run_coding_qualification,
            openagent_sandbox_capability,
            coding_agent_status,
            start_coding_agent
        ]
    };
}

mod tauri {
    pub use crate::tauri_crate::*;
}

mod agent_runtime;
mod artifact_output;
mod coding_agent_gateway;
mod coding_ast;
mod coding_control;
mod coding_delivery;
mod coding_eval;
mod coding_intelligence;
#[path = "coding_lsp_ast.rs"]
mod coding_lsp;
mod coding_patch;
mod connected_agent;
mod connector_ecosystem;
mod connector_input_guard;
mod connector_stabilization;
mod database_backup;
mod dataset_context;
mod dataset_download;
mod document_jobs;
mod folder_opener;
mod github_paging;
mod github_workspace;
mod google_workspace;
mod installation_file;
mod isolated_runtime;
mod local_agent;
mod local_workspace;
mod mcp_response;
mod model_discovery;
mod multimodal;
#[cfg(feature = "native-cxx-llama")]
pub mod native_bridge;
#[cfg(feature = "native-cxx-llama")]
pub mod native_inference;
#[cfg(feature = "native-cxx-llama")]
pub mod native_runtime;
#[cfg(feature = "native-cxx-llama")]
pub mod native_stream;
#[cfg(feature = "native-cxx-llama")]
pub mod native_supervisor;
mod oauth_callback;
mod oauth_refresh;
mod openagent_parallel;
mod openagent_prompt_context;
mod openagent_runs;
mod openagent_security;
mod pdf_ocr;
mod process_ownership;
mod research_documents;
mod research_figure_render;
mod research_projects;
mod research_sections;
mod runtime_guards;
mod scholarly;
mod speech_runtime;
mod text_artifact_save;
mod vision_batch;
mod warm_start;

pub(crate) use agent_runtime::{coding_agent_status, start_coding_agent};
pub(crate) use cloud_bridge::{
    cloud_begin_regeneration, cloud_connect, cloud_disconnect, cloud_download_mesh, cloud_login,
    cloud_request, cloud_status, local_gpu_request, local_gpu_start, local_gpu_stop,
    studio3d_cancel_dependency_install, studio3d_dependency_plan, studio3d_install_dependencies,
};
pub(crate) use coding_control::{approve_coding_action, coding_run_snapshot, reject_coding_action};
pub(crate) use coding_eval::run_coding_qualification;
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
    restore_openagent_checkpoint, resume_coding_run, send_project_agent_message,
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
pub(crate) use openagent_runs::{list_openagent_runs, openagent_run_details};
pub(crate) use openagent_security::openagent_sandbox_capability;
pub(crate) use pdf_ocr::ocr_pdf_pages;
pub(crate) use research_documents::{
    import_local_research_document, list_local_research_documents,
    list_local_research_figure_assets, local_research_figure_data_url,
    register_local_research_figure,
};
pub(crate) use research_projects::{
    accept_local_research_title, approve_local_research_manuscript,
    authorize_local_research_cloud_retry, cancel_local_research_run,
    cancel_local_research_title_job, create_local_research_project,
    export_local_research_academic_text, export_local_research_document,
    export_local_research_markdown, get_local_research_title_job, list_local_research_projects,
    list_local_research_runs, local_research_cloud_model, local_research_run_details,
    local_research_run_for_conversation, pause_local_research_run, resume_local_research_run,
    review_local_research_evidence, save_local_research_manuscript_revision,
    start_local_research_run, suggest_local_research_title, update_local_research_project,
};
pub(crate) use research_sections::{
    approve_local_research_section, assemble_local_research_sections,
    cancel_local_research_section, classify_local_research_document,
    export_local_research_section_manuscript, generate_local_research_section,
    list_local_research_sections, local_research_claim_ledger, local_research_evidence_facts,
    local_research_outline, save_local_research_section_edit,
};
pub(crate) use studio3d::{
    cancel_studio3d_job, create_studio3d_job, create_studio3d_project, delete_studio3d_asset,
    delete_studio3d_scene, duplicate_studio3d_scene, execute_studio3d_local_job,
    import_studio3d_glb, import_studio3d_gltf, import_studio3d_image, import_studio3d_mesh,
    list_studio3d_assets, list_studio3d_jobs, list_studio3d_projects, list_studio3d_scenes,
    load_studio3d_asset, load_studio3d_scene, restore_studio3d_scene_revision, save_studio3d_scene,
    store_studio3d_generated_glb, studio3d_ai_edit_object, studio3d_cloud_edit_object,
    studio3d_neural_capabilities, transition_studio3d_job,
};
pub(crate) use studio_outbox::{
    studio_outbox_claim, studio_outbox_enqueue, studio_outbox_finish, studio_outbox_list,
    studio_outbox_recover, studio_outbox_retry,
};
pub(crate) use warm_start::{prepare_default_chat_runtime, reveal_main_window};

include!("lib_legacy.rs");
