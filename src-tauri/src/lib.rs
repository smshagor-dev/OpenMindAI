mod app_error;
mod artifacts;
mod chat;
mod database;
mod document_generator;
mod github;
mod google;
mod hardware;
mod inference;
mod launch_planner;
mod logging;
mod maintenance;
mod model_catalog;
mod model_download;
mod model_registry;
mod performance;
mod portable_root;
mod projects;
mod runtime;
mod runtime_install;
mod settings;
mod storage;

use std::{path::PathBuf, sync::Mutex};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use artifacts::{Artifact, ArtifactManager, ArtifactRepository, LibraryEntry};
use chat::{ChatRepository, Conversation, Message};
use database::Database;
use github::{GithubAccount, GithubIssueSummary, GithubRepoSummary, GithubRepository};
use google::{GoogleCredentialsStatus, GoogleRepository};
use hardware::{HardwareProfile, HardwareProfiler};
use inference::{ActiveGenerations, InferenceMode, StreamRequest, StreamStartedEvent};
use launch_planner::{LaunchPlan, ModelLaunchPlanner};
use maintenance::{BackupInfo, DiagnosticReport, RepairSummary};
use model_catalog::ModelCatalogReport;
use model_download::{DownloadStatus, ModelDownloadManager};
use model_registry::{ModelLifecycleState, ModelRecord, ModelRegistry};
use performance::{PerformanceProfile, PerformanceProfileManager};
use portable_root::{
    available_bytes_for_path, load_installation, preview_writable, sanitize_profile_name,
    save_installation, InstallConfig, InstallationStatus, PortableRootInfo, PortableRootManager,
    SetupState, RECOMMENDED_INITIAL_STORAGE_BYTES,
};
use projects::{
    project_context_message, Project, ProjectFile, ProjectFileInput, ProjectRepository,
};
use reqwest::Client;
use runtime::{allocate_local_port, LlamaRuntimeManager, LlamaRuntimeStatus, RuntimeInventory};
use runtime_install::{RuntimeInstallStatus, RuntimeInstaller};
use settings::{AppPreferences, SettingsRepository, UserProfile};
use storage::{CacheClearResult, StorageMonitor, StorageSummary};
use tauri::{AppHandle, Emitter, State};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

struct AppState {
    root: PortableRootManager,
    // The database file actually opened at startup -- may differ from
    // `root.database_path()`'s unparameterized default when a first-run
    // profile name was chosen (see `run()`).
    active_database_path: PathBuf,
    database: Mutex<Database>,
    runtime: Mutex<LlamaRuntimeManager>,
    downloads: ModelDownloadManager,
    runtime_installer: RuntimeInstaller,
    active_generations: ActiveGenerations,
    http: Client,
}

#[tauri::command]
fn get_portable_root(state: State<AppState>) -> Result<PortableRootInfo, app_error::AppError> {
    state.root.info(&state.active_database_path)
}

#[tauri::command]
fn installation_status(state: State<AppState>) -> InstallationStatus {
    portable_root::installation_status(&state.root)
}

/// Completes first-run setup: creates the chosen root's directory structure,
/// verifies a database can be opened there under the sanitized profile name,
/// and persists the installation pointer. Takes effect on the next app
/// launch, which re-resolves the root via [`PortableRootManager::resolve`]
/// (no live root/database swap mid-session).
#[tauri::command]
fn complete_setup(
    root: String,
    profile_name: String,
) -> Result<PortableRootInfo, app_error::AppError> {
    // Setup targets the chosen root, not whatever root the currently running
    // dev/preview session happens to be using — no AppState needed.
    let manager = PortableRootManager::from_root(PathBuf::from(root));
    manager.ensure_directories()?;

    let sanitized = sanitize_profile_name(&profile_name);
    let database_filename = format!("{sanitized}.db");
    let database_path = manager.database_path_for_profile(&sanitized);
    Database::open(database_path.clone())?;

    let config = InstallConfig {
        product: "OpenMindAI".to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        root: manager.root().display().to_string(),
        schema_version: portable_root::CURRENT_SCHEMA_VERSION,
        installation_complete: true,
        database: database_filename,
        runtime_ready: false,
        model_ready: false,
        setup_state: Some(SetupState::RootCreated),
    };
    portable_root::save_installation(&config)?;

    tracing::info!(root = %manager.root().display(), "first-run setup completed");
    manager.info(&database_path)
}

/// Persists the user's in-progress storage/profile choice *before* setup
/// finishes, so killing the app mid-wizard resumes at the right step next
/// launch instead of restarting from "welcome" with defaults. Does not touch
/// the filesystem beyond the small `install.json` pointer — no directories
/// are created here (that only happens once in `complete_setup`).
#[tauri::command]
fn save_setup_progress(
    root: String,
    profile_name: String,
    state: SetupState,
) -> Result<(), app_error::AppError> {
    let sanitized = sanitize_profile_name(&profile_name);
    let config = InstallConfig {
        product: "OpenMindAI".to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        root,
        schema_version: portable_root::CURRENT_SCHEMA_VERSION,
        installation_complete: false,
        database: format!("{sanitized}.db"),
        runtime_ready: false,
        model_ready: false,
        setup_state: Some(state),
    };
    tracing::info!(?state, "setup progress saved");
    save_installation(&config)
}

/// Flips the persisted `runtime_ready`/`model_ready` bookkeeping once the
/// live readiness check (`runtime.selected`/model state) actually confirms
/// it — these fields are advisory records of what already happened, never
/// trusted on their own to decide the wizard's state (see `InstallConfig`
/// docs). A missing `install.json` (dev-mode roots with no saved install) is
/// not an error — there's nothing to persist for those.
#[tauri::command]
fn mark_runtime_ready() -> Result<(), app_error::AppError> {
    if let Some(mut config) = load_installation() {
        config.runtime_ready = true;
        config.setup_state = Some(if config.model_ready {
            SetupState::Completed
        } else {
            SetupState::RuntimeReady
        });
        tracing::info!("AI runtime marked ready");
        save_installation(&config)?;
    }
    Ok(())
}

#[tauri::command]
fn mark_model_ready() -> Result<(), app_error::AppError> {
    if let Some(mut config) = load_installation() {
        config.model_ready = true;
        config.setup_state = Some(if config.runtime_ready {
            SetupState::Completed
        } else {
            SetupState::ModelReady
        });
        tracing::info!("AI model marked ready");
        save_installation(&config)?;
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageLocationCheck {
    path: String,
    writable: bool,
    available_bytes: Option<u64>,
    recommended_bytes: u64,
}

/// Non-destructive preview for the setup wizard's Storage screen — does not
/// create any directories, just reports whether the chosen path looks
/// writable and how much free space its disk has.
#[tauri::command]
fn check_storage_location(path: String) -> StorageLocationCheck {
    let candidate = PathBuf::from(&path);
    StorageLocationCheck {
        writable: preview_writable(&candidate),
        available_bytes: available_bytes_for_path(&candidate),
        recommended_bytes: RECOMMENDED_INITIAL_STORAGE_BYTES,
        path,
    }
}

#[tauri::command]
fn list_conversations(state: State<AppState>) -> Result<Vec<Conversation>, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).list_conversations()
}

#[tauri::command]
fn create_conversation(
    title: Option<String>,
    state: State<AppState>,
) -> Result<Conversation, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let repo = ChatRepository::new(&db);
    let conversation = repo.create_conversation(title.as_deref())?;
    let profile = SettingsRepository::new(&db).get_user_profile()?;
    repo.upsert_profile_context(&conversation.id, profile.to_system_context().as_deref())?;
    Ok(conversation)
}

#[tauri::command]
fn rename_conversation(
    id: String,
    title: String,
    state: State<AppState>,
) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).rename_conversation(&id, &title)
}

#[tauri::command]
fn set_conversation_pinned(
    id: String,
    pinned: bool,
    state: State<AppState>,
) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).set_pinned(&id, pinned)
}

#[tauri::command]
fn set_conversation_model(
    id: String,
    model_id: String,
    state: State<AppState>,
) -> Result<Conversation, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ModelRegistry::new(&db, &state.root).validate_model(&model_id)?;
    let repo = ChatRepository::new(&db);
    repo.set_active_model(&id, Some(&model_id))?;
    repo.find_conversation(&id)
}

#[tauri::command]
fn archive_conversation(id: String, state: State<AppState>) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).archive_conversation(&id)
}

#[tauri::command]
fn delete_conversation(id: String, state: State<AppState>) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).delete_conversation(&id)
}

#[tauri::command]
fn list_messages(
    conversation_id: String,
    state: State<AppState>,
) -> Result<Vec<Message>, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).list_messages(&conversation_id)
}

#[tauri::command]
fn add_user_message(
    conversation_id: String,
    content: String,
    state: State<AppState>,
) -> Result<Message, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).add_message(&conversation_id, "user", &content, "completed", None)
}

#[tauri::command]
fn create_streaming_assistant_message(
    conversation_id: String,
    state: State<AppState>,
) -> Result<Message, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).add_message(&conversation_id, "assistant", "", "streaming", None)
}

#[tauri::command]
fn append_message_chunk(
    message_id: String,
    chunk: String,
    state: State<AppState>,
) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).append_message_chunk(&message_id, &chunk)
}

#[tauri::command]
fn complete_message(
    message_id: String,
    status: String,
    state: State<AppState>,
) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).set_message_status(&message_id, &status)
}

#[tauri::command]
fn delete_message(message_id: String, state: State<AppState>) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).delete_message(&message_id)
}

#[tauri::command]
fn list_projects(state: State<AppState>) -> Result<Vec<Project>, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ProjectRepository::new(&db).list_projects()
}

#[tauri::command]
fn create_project(name: String, state: State<AppState>) -> Result<Project, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ProjectRepository::new(&db).create_project(&name)
}

#[tauri::command]
fn update_project(
    project_id: String,
    name: String,
    instructions: String,
    state: State<AppState>,
) -> Result<Project, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ProjectRepository::new(&db).update_project(&project_id, &name, &instructions)
}

#[tauri::command]
fn delete_project(project_id: String, state: State<AppState>) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ProjectRepository::new(&db).delete_project(&project_id)
}

#[tauri::command]
fn link_project_conversation(
    project_id: String,
    conversation_id: String,
    state: State<AppState>,
) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ProjectRepository::new(&db).link_conversation(&project_id, &conversation_id)
}

#[tauri::command]
fn unlink_project_conversation(
    project_id: String,
    conversation_id: String,
    state: State<AppState>,
) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ProjectRepository::new(&db).unlink_conversation(&project_id, &conversation_id)
}

#[tauri::command]
fn add_project_file(
    input: ProjectFileInput,
    state: State<AppState>,
) -> Result<ProjectFile, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ProjectRepository::new(&db).add_file(&input)
}

#[tauri::command]
fn delete_project_file(
    project_id: String,
    file_id: String,
    state: State<AppState>,
) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ProjectRepository::new(&db).delete_file(&project_id, &file_id)
}

#[tauri::command]
fn create_text_artifact(
    conversation_id: String,
    message_id: Option<String>,
    kind: String,
    filename: Option<String>,
    content: String,
    state: State<AppState>,
) -> Result<Artifact, app_error::AppError> {
    if !matches!(kind.as_str(), "text" | "markdown" | "code") {
        return Err(app_error::AppError::internal(
            "unsupported text artifact kind",
        ));
    }

    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let fallback_title = ChatRepository::new(&db)
        .find_conversation(&conversation_id)
        .map(|conversation| conversation.title)
        .unwrap_or_else(|_| "artifact".to_string());

    let manager = ArtifactManager::new(&state.root);
    let (path, filename_final, relative) =
        manager.resolve_destination(&kind, filename.as_deref(), &fallback_title)?;
    std::fs::write(&path, content.as_bytes())?;
    let size = content.len() as i64;

    let artifacts = ArtifactRepository::new(&db);
    let artifact = artifacts.create(
        &conversation_id,
        message_id.as_deref(),
        &filename_final,
        &relative,
        artifacts::mime_type_for(&kind),
        &kind,
        "generating",
    )?;
    artifacts.set_ready(&artifact.id, size, None)?;
    artifacts.find(&artifact.id)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn create_document_artifact(
    app: AppHandle,
    conversation_id: String,
    message_id: Option<String>,
    kind: String,
    filename: Option<String>,
    content: String,
    title: Option<String>,
    state: State<'_, AppState>,
) -> Result<Artifact, app_error::AppError> {
    if !matches!(kind.as_str(), "pdf" | "docx") {
        return Err(app_error::AppError::internal(
            "unsupported document artifact kind",
        ));
    }

    let (artifact, path, document_title) = {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        let repo = ChatRepository::new(&db);
        let fallback_title = title.clone().unwrap_or_else(|| {
            repo.find_conversation(&conversation_id)
                .map(|conversation| conversation.title)
                .unwrap_or_else(|_| "document".to_string())
        });
        let manager = ArtifactManager::new(&state.root);
        let (path, filename_final, relative) =
            manager.resolve_destination(&kind, filename.as_deref(), &fallback_title)?;
        let artifacts = ArtifactRepository::new(&db);
        let artifact = artifacts.create(
            &conversation_id,
            message_id.as_deref(),
            &filename_final,
            &relative,
            artifacts::mime_type_for(&kind),
            &kind,
            "generating",
        )?;
        let document_title = title.unwrap_or_else(|| artifact.name.clone());
        (artifact, path, document_title)
    };

    app.emit("artifact:started", &artifact)
        .map_err(|error| app_error::AppError::internal(error.to_string()))?;

    let generation_result = match kind.as_str() {
        "pdf" => document_generator::generate_pdf(&content, &document_title, &path)
            .map(|meta| Some(meta.page_count)),
        "docx" => document_generator::generate_docx(&content, &document_title, &path).map(|_| None),
        _ => unreachable!(),
    };

    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let artifacts = ArtifactRepository::new(&db);
    let updated = match generation_result {
        Ok(page_count) => {
            let size = std::fs::metadata(&path)
                .map(|metadata| metadata.len() as i64)
                .unwrap_or(0);
            artifacts.set_ready(&artifact.id, size, page_count)?;
            artifacts.find(&artifact.id)?
        }
        Err(error) => {
            artifacts.set_failed(&artifact.id, &error.to_string())?;
            artifacts.find(&artifact.id)?
        }
    };
    drop(db);

    app.emit("artifact:done", &updated)
        .map_err(|error| app_error::AppError::internal(error.to_string()))?;

    Ok(updated)
}

#[tauri::command]
async fn create_generation_artifact(
    app: AppHandle,
    conversation_id: String,
    message_id: Option<String>,
    kind: String,
    prompt: String,
    state: State<'_, AppState>,
) -> Result<Artifact, app_error::AppError> {
    if !matches!(kind.as_str(), "image" | "video" | "voice") {
        return Err(app_error::AppError::internal(
            "unsupported generation artifact kind",
        ));
    }

    let artifact_kind = if kind == "voice" {
        "audio"
    } else {
        kind.as_str()
    };
    let (artifact, path) = {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        let fallback_title = ChatRepository::new(&db)
            .find_conversation(&conversation_id)
            .map(|conversation| conversation.title)
            .unwrap_or_else(|_| "generation".to_string());
        let manager = ArtifactManager::new(&state.root);
        let (path, filename_final, relative) =
            manager.resolve_destination(artifact_kind, None, &fallback_title)?;
        let artifacts = ArtifactRepository::new(&db);
        let artifact = artifacts.create(
            &conversation_id,
            message_id.as_deref(),
            &filename_final,
            &relative,
            artifacts::mime_type_for(artifact_kind),
            artifact_kind,
            "generating",
        )?;
        (artifact, path)
    };

    app.emit("artifact:started", &artifact)
        .map_err(|error| app_error::AppError::internal(error.to_string()))?;

    let generation_result = generate_local_media_artifact(&state, &kind, &prompt, &path);

    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let artifacts = ArtifactRepository::new(&db);
    let updated = match generation_result {
        Ok(()) => {
            let size = std::fs::metadata(&path)
                .map(|metadata| metadata.len() as i64)
                .unwrap_or(0);
            artifacts.set_ready(&artifact.id, size, None)?;
            artifacts.find(&artifact.id)?
        }
        Err(error) => {
            artifacts.set_failed(&artifact.id, &error.to_string())?;
            artifacts.find(&artifact.id)?
        }
    };
    drop(db);

    app.emit("artifact:done", &updated)
        .map_err(|error| app_error::AppError::internal(error.to_string()))?;

    Ok(updated)
}

fn generate_local_media_artifact(
    state: &AppState,
    kind: &str,
    prompt: &str,
    path: &std::path::Path,
) -> Result<(), app_error::AppError> {
    let required_kind = match kind {
        "image" => "image",
        "video" => "video",
        "voice" => "text-to-speech",
        _ => unreachable!(),
    };
    let installed = installed_catalog_entry_for_kind(state, required_kind)?;
    let Some(model) = installed else {
        return Err(app_error::AppError::ArtifactGenerationFailed(format!(
            "{} model download required. Open Settings > Models and download the recommended model first.",
            generation_family_label(kind)
        )));
    };

    match kind {
        "image" => {
            let svg = generation_preview_svg(prompt, &model.entry.name);
            std::fs::write(path, svg.as_bytes())?;
            Ok(())
        }
        "video" => Err(app_error::AppError::ArtifactGenerationFailed(format!(
            "{} is downloaded, but the local video runner is not connected yet. Install the OpenMindAI Motion runtime connector to render MP4 output.",
            model.entry.name
        ))),
        "voice" => Err(app_error::AppError::ArtifactGenerationFailed(format!(
            "{} is downloaded, but the local voice runner is not connected yet. Install the OpenMindAI Speak runtime connector to render WAV output.",
            model.entry.name
        ))),
        _ => unreachable!(),
    }
}

fn installed_catalog_entry_for_kind(
    state: &AppState,
    kind: &str,
) -> Result<Option<model_catalog::ModelCatalogStatus>, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let installed = ModelRegistry::new(&db, &state.root).discover_gguf_models()?;
    drop(db);
    let hardware = HardwareProfiler::detect();
    Ok(
        model_catalog::check_model_updates(&installed, &hardware, &state.root)?
            .entries
            .into_iter()
            .find(|item| item.entry.kind == kind && item.installed),
    )
}

fn generation_family_label(kind: &str) -> &'static str {
    match kind {
        "image" => "OpenMindAI Canvas",
        "video" => "OpenMindAI Motion",
        "voice" => "OpenMindAI Speak",
        _ => "OpenMindAI generation",
    }
}

fn generation_preview_svg(prompt: &str, model_name: &str) -> String {
    let escaped_prompt = escape_xml(prompt.trim());
    let escaped_model = escape_xml(model_name);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720" viewBox="0 0 1280 720">
  <rect width="1280" height="720" fill="#111318"/>
  <rect x="56" y="56" width="1168" height="608" rx="28" fill="#1f232b" stroke="#343a46"/>
  <text x="96" y="132" fill="#f5f7fb" font-family="Segoe UI, Arial, sans-serif" font-size="44" font-weight="700">OpenMindAI Canvas</text>
  <text x="96" y="182" fill="#9aa4b2" font-family="Segoe UI, Arial, sans-serif" font-size="22">Generated local preview with {escaped_model}</text>
  <foreignObject x="96" y="238" width="1088" height="310">
    <div xmlns="http://www.w3.org/1999/xhtml" style="font-family: Segoe UI, Arial, sans-serif; color: #f5f7fb; font-size: 34px; line-height: 1.35; font-weight: 650; overflow-wrap: anywhere;">{escaped_prompt}</div>
  </foreignObject>
  <text x="96" y="610" fill="#7c8796" font-family="Segoe UI, Arial, sans-serif" font-size="18">Full diffusion rendering requires the local image runtime connector.</text>
</svg>"##
    )
}

fn escape_xml(input: &str) -> String {
    input
        .chars()
        .flat_map(|character| match character {
            '&' => "&amp;".chars().collect::<Vec<_>>(),
            '<' => "&lt;".chars().collect::<Vec<_>>(),
            '>' => "&gt;".chars().collect::<Vec<_>>(),
            '"' => "&quot;".chars().collect::<Vec<_>>(),
            '\'' => "&apos;".chars().collect::<Vec<_>>(),
            other => vec![other],
        })
        .collect()
}

#[tauri::command]
fn list_artifacts(
    conversation_id: String,
    state: State<AppState>,
) -> Result<Vec<Artifact>, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ArtifactRepository::new(&db).list_for_conversation(&conversation_id)
}

#[tauri::command]
fn list_library_entries(state: State<AppState>) -> Result<Vec<LibraryEntry>, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ArtifactRepository::new(&db).list_all(200)
}

#[tauri::command]
fn open_artifact(artifact_id: String, state: State<AppState>) -> Result<(), app_error::AppError> {
    let path = {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        let artifact = ArtifactRepository::new(&db).find(&artifact_id)?;
        state.root.resolve_relative(&artifact.path)?
    };
    let mut command = std::process::Command::new("cmd");
    command.args(["/C", "start", "", &path.display().to_string()]);
    hide_console_window(&mut command);
    command
        .spawn()
        .map_err(|error| app_error::AppError::internal(error.to_string()))?;
    Ok(())
}

fn hide_console_window(_command: &mut std::process::Command) {
    #[cfg(target_os = "windows")]
    {
        _command.creation_flags(CREATE_NO_WINDOW);
    }
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), app_error::AppError> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(app_error::AppError::internal("unsupported external URL"));
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("rundll32");
        command.args(["url.dll,FileProtocolHandler", &url]);
        hide_console_window(&mut command);
        command
            .spawn()
            .map_err(|error| app_error::AppError::internal(error.to_string()))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|error| app_error::AppError::internal(error.to_string()))?;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|error| app_error::AppError::internal(error.to_string()))?;
    }

    Ok(())
}

#[tauri::command]
fn reveal_artifact_in_folder(
    artifact_id: String,
    state: State<AppState>,
) -> Result<(), app_error::AppError> {
    let path = {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        let artifact = ArtifactRepository::new(&db).find(&artifact_id)?;
        state.root.resolve_relative(&artifact.path)?
    };
    std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn()
        .map_err(|error| app_error::AppError::internal(error.to_string()))?;
    Ok(())
}

#[tauri::command]
fn detect_hardware() -> HardwareProfile {
    HardwareProfiler::detect()
}

#[tauri::command]
fn get_performance_profile() -> PerformanceProfile {
    let hardware = HardwareProfiler::detect();
    PerformanceProfileManager::auto(&hardware)
}

#[tauri::command]
fn discover_models(state: State<AppState>) -> Result<Vec<ModelRecord>, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ModelRegistry::new(&db, &state.root).discover_gguf_models()
}

#[tauri::command]
fn get_qwen_download_status(state: State<AppState>) -> Result<DownloadStatus, app_error::AppError> {
    state.downloads.status()
}

#[tauri::command]
fn get_model_download_status(
    state: State<AppState>,
) -> Result<DownloadStatus, app_error::AppError> {
    state.downloads.status()
}

#[tauri::command]
async fn download_qwen_model(
    state: State<'_, AppState>,
) -> Result<DownloadStatus, app_error::AppError> {
    let status = state.downloads.download_qwen_q4_k_m().await?;
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ModelRegistry::new(&db, &state.root).discover_gguf_models()?;
    Ok(status)
}

#[tauri::command]
async fn download_catalog_model(
    model_id: String,
    state: State<'_, AppState>,
) -> Result<DownloadStatus, app_error::AppError> {
    let status = state.downloads.download_catalog_model(&model_id).await?;
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ModelRegistry::new(&db, &state.root).discover_gguf_models()?;
    Ok(status)
}

#[tauri::command]
fn cancel_qwen_download(state: State<AppState>) -> Result<DownloadStatus, app_error::AppError> {
    state.downloads.cancel()
}

#[tauri::command]
fn cancel_model_download(state: State<AppState>) -> Result<DownloadStatus, app_error::AppError> {
    state.downloads.cancel()
}

#[tauri::command]
fn pause_model_download(state: State<AppState>) -> Result<DownloadStatus, app_error::AppError> {
    state.downloads.pause()
}

#[tauri::command]
fn delete_catalog_model(
    model_id: String,
    state: State<AppState>,
) -> Result<(), app_error::AppError> {
    let entry = model_catalog::entry_by_id(&model_id)?;
    model_catalog::delete_catalog_model(&state.root, &model_id)?;
    if let Some(download) = entry.download {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        ModelRegistry::new(&db, &state.root).remove_by_path_prefix(&download.destination_dir)?;
    }
    Ok(())
}

#[tauri::command]
fn validate_model(
    model_id: String,
    state: State<AppState>,
) -> Result<ModelRecord, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ModelRegistry::new(&db, &state.root).validate_model(&model_id)
}

#[tauri::command]
fn plan_model_launch(
    model_id: String,
    state: State<AppState>,
) -> Result<LaunchPlan, app_error::AppError> {
    let hardware = HardwareProfiler::detect();
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let model = ModelRegistry::new(&db, &state.root).validate_model(&model_id)?;
    let port = allocate_local_port()?;
    Ok(ModelLaunchPlanner::plan(&model, &hardware, port))
}

#[tauri::command]
async fn activate_model(
    conversation_id: String,
    model_id: String,
    state: State<'_, AppState>,
) -> Result<LlamaRuntimeStatus, app_error::AppError> {
    let model = {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        ModelRegistry::new(&db, &state.root).validate_model(&model_id)?
    };
    let hardware = HardwareProfiler::detect();
    let plan = ModelLaunchPlanner::plan(&model, &hardware, allocate_local_port()?);
    let status = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| app_error::AppError::internal("runtime lock poisoned"))?;
        runtime.ensure_model_server(&hardware, &plan.config)?
    };

    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ChatRepository::new(&db).set_active_model(&conversation_id, Some(&model_id))?;
    Ok(status)
}

#[tauri::command]
fn get_storage_summary(state: State<AppState>) -> Result<StorageSummary, app_error::AppError> {
    StorageMonitor::new(&state.root).summary()
}

#[tauri::command]
fn clear_cache(state: State<AppState>) -> Result<CacheClearResult, app_error::AppError> {
    let result = StorageMonitor::new(&state.root).clear_cache()?;
    tracing::info!(bytes_freed = result.bytes_freed, "cache cleared");
    Ok(result)
}

#[tauri::command]
fn run_diagnostics(state: State<AppState>) -> Result<DiagnosticReport, app_error::AppError> {
    let hardware = HardwareProfiler::detect();
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let runtime = state
        .runtime
        .lock()
        .map_err(|_| app_error::AppError::internal("runtime lock poisoned"))?;
    maintenance::run_diagnostics(&state.root, &db, &hardware, &runtime)
}

/// Re-runs the same idempotent operations already proven in setup
/// (`ensure_directories`, runtime auto-install, model download) rather than
/// inventing new recovery logic. Never holds a database/runtime lock across
/// an `.await` point (mirrors `download_qwen_model`'s existing pattern).
#[tauri::command]
async fn repair_installation(
    state: State<'_, AppState>,
) -> Result<RepairSummary, app_error::AppError> {
    let hardware = HardwareProfiler::detect();
    let mut actions = Vec::new();

    state.root.ensure_directories()?;
    actions.push("Verified required folders exist".to_string());

    let runtime_missing = {
        let runtime = state
            .runtime
            .lock()
            .map_err(|_| app_error::AppError::internal("runtime lock poisoned"))?;
        runtime.inventory(&hardware)?.selected.is_none()
    };
    if runtime_missing {
        match state.runtime_installer.install_recommended(&hardware).await {
            Ok(status) => actions.push(format!(
                "Installed AI engine ({})",
                status
                    .backend
                    .map(|backend| format!("{backend:?}"))
                    .unwrap_or_else(|| "unknown backend".to_string())
            )),
            Err(error) => actions.push(format!(
                "Could not install AI engine automatically: {error}"
            )),
        }
    } else {
        actions.push("AI engine already installed".to_string());
    }

    let model_missing = {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        !ModelRegistry::new(&db, &state.root)
            .discover_gguf_models()?
            .iter()
            .any(|model| model.state == ModelLifecycleState::Ready)
    };
    if model_missing {
        match state.downloads.download_qwen_q4_k_m().await {
            Ok(_) => actions.push("Downloaded and verified the AI model".to_string()),
            Err(error) => actions.push(format!(
                "Could not download the AI model automatically: {error}"
            )),
        }
    } else {
        actions.push("AI model already installed".to_string());
    }

    tracing::info!(?actions, "repair completed");
    Ok(RepairSummary { actions })
}

#[tauri::command]
fn backup_database(state: State<AppState>) -> Result<BackupInfo, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let backup = maintenance::backup_database(&state.root, &db)?;
    tracing::info!(name = %backup.name, size_bytes = backup.size_bytes, "database backup created");
    Ok(backup)
}

#[tauri::command]
fn list_backups(state: State<AppState>) -> Result<Vec<BackupInfo>, app_error::AppError> {
    maintenance::list_backups(&state.root)
}

#[tauri::command]
fn check_model_updates(state: State<AppState>) -> Result<ModelCatalogReport, app_error::AppError> {
    let hardware = HardwareProfiler::detect();
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let installed = ModelRegistry::new(&db, &state.root).discover_gguf_models()?;
    let report = model_catalog::check_model_updates(&installed, &hardware, &state.root)?;
    let updates = report
        .entries
        .iter()
        .filter(|status| status.update_available)
        .count();
    tracing::info!(
        entries = report.entries.len(),
        updates_available = updates,
        "model catalog checked"
    );
    Ok(report)
}

#[tauri::command]
fn open_maintenance_folder(
    folder: String,
    state: State<AppState>,
) -> Result<(), app_error::AppError> {
    let relative = match folder.as_str() {
        "logs" => "logs",
        "backups" => "backups",
        _ => return Err(app_error::AppError::internal("unknown maintenance folder")),
    };
    maintenance::open_folder_in_root(&state.root, relative)
}

#[tauri::command]
fn read_recent_logs(state: State<AppState>) -> Result<String, app_error::AppError> {
    logging::read_recent(&state.root, 200)
}

#[tauri::command]
fn get_llama_runtime_status(
    state: State<AppState>,
) -> Result<LlamaRuntimeStatus, app_error::AppError> {
    let hardware = HardwareProfiler::detect();
    let runtime = state
        .runtime
        .lock()
        .map_err(|_| app_error::AppError::internal("runtime lock poisoned"))?;
    runtime.status(&hardware)
}

#[tauri::command]
fn get_llama_runtime_inventory(
    state: State<AppState>,
) -> Result<RuntimeInventory, app_error::AppError> {
    let hardware = HardwareProfiler::detect();
    let runtime = state
        .runtime
        .lock()
        .map_err(|_| app_error::AppError::internal("runtime lock poisoned"))?;
    runtime.inventory(&hardware)
}

#[tauri::command]
fn get_runtime_install_status(
    state: State<AppState>,
) -> Result<RuntimeInstallStatus, app_error::AppError> {
    state.runtime_installer.status()
}

#[tauri::command]
async fn install_recommended_runtime(
    state: State<'_, AppState>,
) -> Result<RuntimeInstallStatus, app_error::AppError> {
    let hardware = HardwareProfiler::detect();
    state.runtime_installer.install_recommended(&hardware).await
}

#[tauri::command]
fn cancel_runtime_install(
    state: State<AppState>,
) -> Result<RuntimeInstallStatus, app_error::AppError> {
    state.runtime_installer.cancel()
}

#[tauri::command]
fn start_llama_runtime(state: State<AppState>) -> Result<LlamaRuntimeStatus, app_error::AppError> {
    let hardware = HardwareProfiler::detect();
    let mut runtime = state
        .runtime
        .lock()
        .map_err(|_| app_error::AppError::internal("runtime lock poisoned"))?;
    runtime.start_server(&hardware)
}

#[tauri::command]
fn stop_llama_runtime(state: State<AppState>) -> Result<(), app_error::AppError> {
    let mut runtime = state
        .runtime
        .lock()
        .map_err(|_| app_error::AppError::internal("runtime lock poisoned"))?;
    runtime.stop()
}

fn resolve_conversation_model(
    state: &State<'_, AppState>,
    conversation_id: &str,
    mode: &str,
    content: &str,
) -> Result<ModelRoutingDecision, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let repo = ChatRepository::new(&db);
    let registry = ModelRegistry::new(&db, &state.root);
    let models = registry.discover_gguf_models()?;

    let active_model_id = repo
        .find_conversation(conversation_id)
        .ok()
        .and_then(|conversation| conversation.active_model_id);
    let selected = select_conversation_model(&models, active_model_id.as_deref(), mode, content)
        .ok_or_else(|| {
            app_error::AppError::ModelNotFound(
                "OpenMindAI Core is not installed. Download it from Settings > Models first."
                    .to_string(),
            )
        })?;
    let reason = routing_reason(&selected, active_model_id.as_deref(), mode, content);

    if active_model_id.as_deref() != Some(selected.id.as_str()) {
        repo.set_active_model(conversation_id, Some(&selected.id))?;
    }
    Ok(ModelRoutingDecision {
        model: selected,
        reason,
    })
}

struct ModelRoutingDecision {
    model: ModelRecord,
    reason: String,
}

async fn run_streaming_completion(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation_id: &str,
    model: &ModelRecord,
    assistant: &Message,
    mode: &str,
) -> Result<(), app_error::AppError> {
    let hardware = HardwareProfiler::detect();
    let plan = ModelLaunchPlanner::plan(model, &hardware, allocate_local_port()?);
    let endpoint = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| app_error::AppError::internal("runtime lock poisoned"))?;
        runtime.ensure_model_server(&hardware, &plan.config)?;
        runtime.status(&hardware)?.endpoint.ok_or_else(|| {
            app_error::AppError::InferenceServerUnavailable("runtime endpoint missing".to_string())
        })?
    };

    let inference_mode = if mode.eq_ignore_ascii_case("thinking") {
        InferenceMode::Thinking
    } else {
        InferenceMode::Chat
    };
    let result = inference::stream_chat_completion(StreamRequest {
        app,
        database: &state.database,
        active: &state.active_generations,
        client: &state.http,
        endpoint: &endpoint,
        conversation_id,
        assistant,
        mode: inference_mode,
    })
    .await;

    if let Err(error) = result {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        let repo = ChatRepository::new(&db);
        let _ = repo.set_message_status(&assistant.id, "failed");
        state.active_generations.finish(conversation_id);
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
async fn send_chat_message(
    app: AppHandle,
    conversation_id: String,
    content: String,
    mode: String,
    state: State<'_, AppState>,
) -> Result<Message, app_error::AppError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(app_error::AppError::InferenceFailed(
            "message cannot be empty".to_string(),
        ));
    }

    let routing = resolve_conversation_model(&state, &conversation_id, &mode, trimmed)?;
    let model = routing.model;

    let (user, assistant) = {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        let repo = ChatRepository::new(&db);
        let user = repo.add_message(
            &conversation_id,
            "user",
            trimmed,
            "completed",
            Some(&model.id),
        )?;
        let assistant = repo.add_message(
            &conversation_id,
            "assistant",
            "",
            "streaming",
            Some(&model.id),
        )?;
        (user, assistant)
    };
    sync_project_context(&state, &conversation_id)?;
    app.emit(
        "inference:started",
        StreamStartedEvent {
            conversation_id: conversation_id.clone(),
            user,
            assistant: assistant.clone(),
            routed_model_name: model.name.clone(),
            routing_reason: routing.reason,
        },
    )
    .map_err(|error| app_error::AppError::StreamFailed(error.to_string()))?;

    run_streaming_completion(&app, &state, &conversation_id, &model, &assistant, &mode).await?;
    Ok(assistant)
}

#[tauri::command]
async fn regenerate_message(
    app: AppHandle,
    conversation_id: String,
    assistant_message_id: String,
    mode: String,
    state: State<'_, AppState>,
) -> Result<Message, app_error::AppError> {
    let user = {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        let repo = ChatRepository::new(&db);
        let history = repo.list_messages(&conversation_id)?;
        let target_index = history
            .iter()
            .position(|message| message.id == assistant_message_id)
            .ok_or_else(|| app_error::AppError::internal("assistant message not found"))?;
        let user = history[..target_index]
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .cloned()
            .ok_or_else(|| {
                app_error::AppError::internal("no preceding user message to regenerate from")
            })?;

        repo.delete_message(&assistant_message_id)?;
        user
    };
    let routing = resolve_conversation_model(&state, &conversation_id, &mode, &user.content)?;
    let model = routing.model;
    sync_project_context(&state, &conversation_id)?;
    let assistant = {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        ChatRepository::new(&db).add_message(
            &conversation_id,
            "assistant",
            "",
            "streaming",
            Some(&model.id),
        )?
    };
    app.emit(
        "inference:started",
        StreamStartedEvent {
            conversation_id: conversation_id.clone(),
            user,
            assistant: assistant.clone(),
            routed_model_name: model.name.clone(),
            routing_reason: routing.reason,
        },
    )
    .map_err(|error| app_error::AppError::StreamFailed(error.to_string()))?;

    run_streaming_completion(&app, &state, &conversation_id, &model, &assistant, &mode).await?;
    Ok(assistant)
}

fn select_conversation_model(
    models: &[ModelRecord],
    active_model_id: Option<&str>,
    mode: &str,
    content: &str,
) -> Option<ModelRecord> {
    let normalized_mode = mode.to_ascii_lowercase();
    let normalized_content = content.to_ascii_lowercase();

    if matches!(normalized_mode.as_str(), "vision") {
        if let Some(model) = model_by_repo(models, "ggml-org/Qwen2.5-VL-3B-Instruct-GGUF") {
            return Some(model);
        }
    }

    if matches!(
        normalized_mode.as_str(),
        "thinking" | "research" | "search" | "image" | "video" | "voice"
    ) || looks_like_code_task(&normalized_content)
    {
        if let Some(model) = model_by_repo(models, "Qwen/Qwen3-8B-GGUF") {
            return Some(model);
        }
    }

    if normalized_mode == "chat" {
        if let Some(model) =
            active_model_id.and_then(|id| models.iter().find(|model| model.id == id))
        {
            return Some(model.clone());
        }
    }

    model_by_repo(models, "Qwen/Qwen3-4B-GGUF")
        .or_else(|| model_by_repo(models, "Qwen/Qwen3-8B-GGUF"))
        .or_else(|| models.iter().find(|model| model.enabled).cloned())
}

fn model_by_repo(models: &[ModelRecord], repo: &str) -> Option<ModelRecord> {
    models
        .iter()
        .find(|model| {
            model.enabled
                && model
                    .source_repository
                    .as_deref()
                    .is_some_and(|source| source == repo)
        })
        .cloned()
}

fn looks_like_code_task(content: &str) -> bool {
    [
        "code",
        "debug",
        "bug",
        "typescript",
        "javascript",
        "rust",
        "python",
        "react",
        "tauri",
        "function",
        "compile",
        "error",
        "stack trace",
    ]
    .iter()
    .any(|needle| content.contains(needle))
}

fn routing_reason(
    selected: &ModelRecord,
    active_model_id: Option<&str>,
    mode: &str,
    content: &str,
) -> String {
    let normalized_mode = mode.to_ascii_lowercase();
    if active_model_id == Some(selected.id.as_str()) && normalized_mode == "chat" {
        return format!("Using your selected model: {}", selected.name);
    }
    match normalized_mode.as_str() {
        "vision" => format!("Routed visual input to {}", selected.name),
        "thinking" => format!("Routed reasoning task to {}", selected.name),
        "research" | "search" => format!("Routed research/search task to {}", selected.name),
        "image" => format!("Routed image request to {}", selected.name),
        "video" => format!("Routed video request to {}", selected.name),
        "voice" => format!("Routed voice request to {}", selected.name),
        _ if looks_like_code_task(&content.to_ascii_lowercase()) => {
            format!("Routed coding task to {}", selected.name)
        }
        _ => format!("Routed chat to {}", selected.name),
    }
}

fn sync_project_context(
    state: &State<'_, AppState>,
    conversation_id: &str,
) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let project = ProjectRepository::new(&db).project_for_conversation(conversation_id)?;
    let content = project.as_ref().and_then(project_context_message);
    ChatRepository::new(&db).upsert_profile_context(conversation_id, content.as_deref())
}

#[tauri::command]
fn cancel_generation(
    conversation_id: String,
    state: State<AppState>,
) -> Result<(), app_error::AppError> {
    state.active_generations.cancel(&conversation_id)
}

#[tauri::command]
fn get_app_preferences(state: State<AppState>) -> Result<AppPreferences, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    SettingsRepository::new(&db).get_preferences()
}

#[tauri::command]
fn save_app_preferences(
    preferences: AppPreferences,
    state: State<AppState>,
) -> Result<AppPreferences, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    SettingsRepository::new(&db).save_preferences(&preferences)
}

#[tauri::command]
fn get_user_profile(state: State<AppState>) -> Result<UserProfile, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    SettingsRepository::new(&db).get_user_profile()
}

#[tauri::command]
fn save_user_profile(
    profile: UserProfile,
    state: State<AppState>,
) -> Result<UserProfile, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    let saved = SettingsRepository::new(&db).save_user_profile(&profile)?;
    let context = saved.to_system_context();
    let repo = ChatRepository::new(&db);
    for conversation in repo.list_conversations()? {
        repo.upsert_profile_context(&conversation.id, context.as_deref())?;
    }
    Ok(saved)
}

#[tauri::command]
fn get_github_account(
    state: State<AppState>,
) -> Result<Option<GithubAccount>, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    GithubRepository::new(&db).get_account()
}

#[tauri::command]
async fn save_github_token(
    token: String,
    state: State<'_, AppState>,
) -> Result<GithubAccount, app_error::AppError> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(app_error::AppError::GithubApiError(
            "token cannot be empty".to_string(),
        ));
    }
    let account = github::fetch_account(&state.http, trimmed).await?;
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    GithubRepository::new(&db).save(trimmed, &account)?;
    Ok(account)
}

#[tauri::command]
fn disconnect_github(state: State<AppState>) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    GithubRepository::new(&db).clear()
}

#[tauri::command]
async fn list_github_repos(
    state: State<'_, AppState>,
) -> Result<Vec<GithubRepoSummary>, app_error::AppError> {
    let token = {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        GithubRepository::new(&db).get_token()?
    };
    let token = token.ok_or_else(|| {
        app_error::AppError::GithubApiError("GitHub is not connected".to_string())
    })?;
    github::fetch_repos(&state.http, &token).await
}

#[tauri::command]
async fn list_github_issues(
    repo_full_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<GithubIssueSummary>, app_error::AppError> {
    let token = {
        let db = state
            .database
            .lock()
            .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
        GithubRepository::new(&db).get_token()?
    };
    let token = token.ok_or_else(|| {
        app_error::AppError::GithubApiError("GitHub is not connected".to_string())
    })?;
    github::fetch_issues(&state.http, &token, &repo_full_name).await
}

#[tauri::command]
fn get_google_credentials(
    state: State<AppState>,
) -> Result<Option<GoogleCredentialsStatus>, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    GoogleRepository::new(&db).get_status()
}

#[tauri::command]
fn save_google_credentials(
    client_id: String,
    client_secret: String,
    state: State<AppState>,
) -> Result<GoogleCredentialsStatus, app_error::AppError> {
    let client_id = client_id.trim();
    let client_secret = client_secret.trim();
    if client_id.is_empty() || client_secret.is_empty() {
        return Err(app_error::AppError::internal(
            "Client ID and Client Secret are both required",
        ));
    }
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    GoogleRepository::new(&db).save(client_id, client_secret)
}

#[tauri::command]
fn clear_google_credentials(state: State<AppState>) -> Result<(), app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    GoogleRepository::new(&db).clear()
}

pub fn run() {
    let root = PortableRootManager::resolve().expect("failed to resolve OpenMindAI Root");
    root.ensure_directories()
        .expect("failed to initialize OpenMindAI directories");
    // Held for the rest of this function's scope (which doesn't return
    // until the app exits, via tauri::Builder::run below) so log lines
    // keep flushing for the whole session.
    let _log_guard = logging::init(&root);
    tracing::info!(root = %root.root().display(), mode = ?root.mode(), "resolved OpenMindAI root");
    // When this root was set up via the first-run wizard, honor the
    // profile-named database it configured rather than the dev-mode default.
    let database_path = portable_root::load_installation()
        .filter(|config| {
            config.installation_complete && config.root == root.root().display().to_string()
        })
        .map(|config| {
            root.root()
                .join("data")
                .join("database")
                .join(config.database)
        })
        .unwrap_or_else(|| root.database_path());
    let database =
        Database::open(database_path.clone()).expect("failed to initialize SQLite database");
    let runtime = LlamaRuntimeManager::new(root.clone());
    let downloads = ModelDownloadManager::new(root.clone());
    let runtime_installer = RuntimeInstaller::new(root.clone());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState {
            root,
            active_database_path: database_path,
            database: Mutex::new(database),
            runtime: Mutex::new(runtime),
            downloads,
            runtime_installer,
            active_generations: ActiveGenerations::default(),
            http: Client::new(),
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
        .run(tauri::generate_context!())
        .expect("error while running OpenMindAI");
}

#[cfg(test)]
mod milestone2_tests {
    use super::*;

    #[test]
    fn actual_or_temp_root_persistence_round_trip() {
        let root = PortableRootManager::resolve().unwrap_or_else(|_| {
            PortableRootManager::from_root(tempfile::tempdir().unwrap().path().join("OpenMindAI"))
        });
        root.ensure_directories().unwrap();
        let db_path = root.database_path();
        let conversation_id;

        {
            let database = Database::open(db_path.clone()).unwrap();
            let repo = ChatRepository::new(&database);
            let conversation = repo
                .create_conversation(Some("Milestone 2 persistence smoke"))
                .unwrap();
            conversation_id = conversation.id.clone();
            repo.add_message(&conversation.id, "user", "persist user", "completed", None)
                .unwrap();
            let assistant = repo
                .add_message(&conversation.id, "assistant", "", "streaming", None)
                .unwrap();
            repo.append_message_chunk(&assistant.id, "persist assistant")
                .unwrap();
            repo.set_message_status(&assistant.id, "cancelled").unwrap();
        }

        let database = Database::open(db_path).unwrap();
        let repo = ChatRepository::new(&database);
        let messages = repo.list_messages(&conversation_id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "persist user");
        assert_eq!(messages[1].content, "persist assistant");
        assert_eq!(messages[1].status, "cancelled");
    }

    fn test_model(id: &str, source_repository: Option<&str>, enabled: bool) -> ModelRecord {
        ModelRecord {
            id: id.to_string(),
            name: id.to_string(),
            family: None,
            path: format!("models/llm/{id}.gguf"),
            format: "gguf".to_string(),
            quantization: None,
            size_bytes: 0,
            capabilities: "[\"chat\"]".to_string(),
            context_length: None,
            preferred_backend: None,
            enabled,
            source_repository: source_repository.map(ToString::to_string),
            verification: None,
            state: model_registry::ModelLifecycleState::Ready,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
        }
    }

    #[test]
    fn select_conversation_model_prefers_active_binding() {
        let qwen = test_model("qwen", Some("Qwen/Qwen3-4B-GGUF"), true);
        let other = test_model("other", None, true);
        let models = vec![qwen.clone(), other.clone()];

        let selected = select_conversation_model(&models, Some("other"), "chat", "hello").unwrap();
        assert_eq!(selected.id, "other");
    }

    #[test]
    fn select_conversation_model_falls_back_to_qwen_then_enabled() {
        let qwen = test_model("qwen", Some("Qwen/Qwen3-4B-GGUF"), true);
        let other = test_model("other", None, true);
        let models = vec![other.clone(), qwen.clone()];

        let selected = select_conversation_model(&models, None, "chat", "hello").unwrap();
        assert_eq!(selected.id, "qwen");

        let models_without_qwen = vec![other.clone()];
        let fallback =
            select_conversation_model(&models_without_qwen, Some("missing"), "chat", "hello")
                .unwrap();
        assert_eq!(fallback.id, "other");

        let no_models: Vec<ModelRecord> = Vec::new();
        assert!(select_conversation_model(&no_models, None, "chat", "hello").is_none());
    }

    #[test]
    fn select_conversation_model_routes_code_to_titan() {
        let core = test_model("core", Some("Qwen/Qwen3-4B-GGUF"), true);
        let titan = test_model("titan", Some("Qwen/Qwen3-8B-GGUF"), true);
        let models = vec![core, titan.clone()];

        let selected =
            select_conversation_model(&models, None, "chat", "debug this rust compile error")
                .unwrap();

        assert_eq!(selected.id, titan.id);
    }

    #[test]
    fn select_conversation_model_routes_vision_to_lens() {
        let core = test_model("core", Some("Qwen/Qwen3-4B-GGUF"), true);
        let lens = test_model("lens", Some("ggml-org/Qwen2.5-VL-3B-Instruct-GGUF"), true);
        let models = vec![core, lens.clone()];

        let selected =
            select_conversation_model(&models, None, "vision", "look at this image").unwrap();

        assert_eq!(selected.id, lens.id);
    }
}
