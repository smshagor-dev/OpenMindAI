from pathlib import Path


def replace_exact(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"anchor missing in {path}: {old[:120]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


replace_exact(
    "src-tauri/src/database.rs",
    '''    Migration {
        number: 8,
        name: "008_openagent_restore_audit",
        sql: include_str!("../migrations/008_openagent_restore_audit.sql"),
    },
];''',
    '''    Migration {
        number: 8,
        name: "008_openagent_restore_audit",
        sql: include_str!("../migrations/008_openagent_restore_audit.sql"),
    },
    Migration {
        number: 9,
        name: "009_coding_workspace_control",
        sql: include_str!("../migrations/009_coding_workspace_control.sql"),
    },
];''',
)

replace_exact(
    "src-tauri/src/portable_root.rs",
    "pub const CURRENT_SCHEMA_VERSION: u32 = 8;",
    "pub const CURRENT_SCHEMA_VERSION: u32 = 9;",
)

replace_exact(
    "src-tauri/src/lib.rs",
    '''            restore_openagent_checkpoint,
            send_project_agent_message,''',
    '''            restore_openagent_checkpoint,
            resume_project_agent_run,
            send_project_agent_message,''',
)
replace_exact(
    "src-tauri/src/lib.rs",
    '''mod coding_lsp;
mod coding_patch;''',
    '''mod coding_control;
mod coding_lsp;
mod coding_patch;''',
)
replace_exact(
    "src-tauri/src/lib.rs",
    '''pub(crate) use local_agent::{
    project_agent_status_for_conversation, regenerate_project_agent_message,
    restore_openagent_checkpoint, send_project_agent_message,
};''',
    '''pub(crate) use local_agent::{
    project_agent_status_for_conversation, regenerate_project_agent_message,
    restore_openagent_checkpoint, resume_project_agent_run, send_project_agent_message,
};''',
)

replace_exact(
    "src-tauri/src/local_agent.rs",
    '''    chat::{ChatRepository, Message},
    coding_lsp, coding_patch,
    database::Database,''',
    '''    chat::{ChatRepository, Message},
    coding_control, coding_lsp, coding_patch,
    database::Database,''',
)
replace_exact(
    "src-tauri/src/local_agent.rs",
    '''    run_agent_message(&app, &state, &conversation_id, &content, None).await''',
    '''    run_agent_message(&app, &state, &conversation_id, &content, None, None).await''',
)
replace_exact(
    "src-tauri/src/local_agent.rs",
    '''    run_agent_message(&app, &state, &conversation_id, &content, Some(user)).await
}

#[tauri::command]
pub fn restore_openagent_checkpoint(''',
    '''    run_agent_message(
        &app,
        &state,
        &conversation_id,
        &content,
        Some(user),
        None,
    )
    .await
}

#[tauri::command]
pub async fn resume_project_agent_run(
    app: AppHandle,
    run_id: String,
    state: State<'_, AppState>,
) -> Result<Message, AppError> {
    let (run, user) = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        let run = OpenAgentRunRepository::new(&db)
            .find(&run_id)?
            .ok_or_else(|| AppError::internal("interrupted OpenAgent run not found"))?;
        if run.status != "interrupted" {
            return Err(AppError::internal(
                "only an interrupted OpenAgent run can be resumed",
            ));
        }
        let repo = ChatRepository::new(&db);
        let messages = repo.list_messages(&run.conversation_id)?;
        let target_index = messages
            .iter()
            .position(|message| message.id == run.assistant_message_id)
            .ok_or_else(|| AppError::internal("interrupted run assistant message not found"))?;
        let user = messages[..target_index]
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .cloned()
            .ok_or_else(|| AppError::internal("interrupted run user message not found"))?;
        (run, user)
    };

    let content = run.goal.clone();
    let conversation_id = run.conversation_id.clone();
    let parent_run_id = run.id.clone();
    run_agent_message(
        &app,
        &state,
        &conversation_id,
        &content,
        Some(user),
        Some(&parent_run_id),
    )
    .await
}

#[tauri::command]
pub fn restore_openagent_checkpoint(''',
)
replace_exact(
    "src-tauri/src/local_agent.rs",
    '''async fn run_agent_message(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation_id: &str,
    content: &str,
    existing_user: Option<Message>,
) -> Result<Message, AppError> {''',
    '''async fn run_agent_message(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation_id: &str,
    content: &str,
    existing_user: Option<Message>,
    resume_parent: Option<&str>,
) -> Result<Message, AppError> {''',
)
replace_exact(
    "src-tauri/src/local_agent.rs",
    '''            )?
            .id
    };

    if let Err(error) = app.emit(''',
    '''            )?
            .id
    };
    let (resume_seed, inherited_validation_required) = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        coding_control::initialize_run(&db, &run_id, content, "{}")?;
        if let Some(parent_run_id) = resume_parent {
            let parent = OpenAgentRunRepository::new(&db)
                .find(parent_run_id)?
                .ok_or_else(|| AppError::internal("resume parent OpenAgent run not found"))?;
            coding_control::link_runs(&db, parent_run_id, &run_id)?;
            (
                Some(coding_control::resume_seed(&db, parent_run_id)?),
                parent.validation_status == "required",
            )
        } else {
            (None, false)
        }
    };

    if let Err(error) = app.emit(''',
)
replace_exact(
    "src-tauri/src/local_agent.rs",
    '''    let mut transcript = VecDeque::<String>::new();
    let mut consecutive_failures = 0usize;
    let mut last_action_signature: Option<String> = None;
    let mut identical_action_repeats = 0usize;
    let mut validation_required = false;''',
    '''    let mut transcript = VecDeque::<String>::new();
    if let Some(seed) = resume_seed {
        push_transcript(&mut transcript, seed);
    }
    let mut consecutive_failures = 0usize;
    let mut last_action_signature: Option<String> = None;
    let mut identical_action_repeats = 0usize;
    let mut validation_required = inherited_validation_required;''',
)
replace_exact(
    "src-tauri/src/local_agent.rs",
    '''    let intro = format!(
        "OpenAgent started for **{}** using **{}**. I can inspect and change the attached workspace{}.",''',
    '''    let intro = format!(
        "OpenAgent {} for **{}** using **{}**. I can inspect and change the attached workspace{}.",
        if resume_parent.is_some() { "resumed" } else { "started" },''',
)
replace_exact(
    "src-tauri/src/local_agent.rs",
    '''        agent_context.project.name,
        model.name,
        if agent_context.workspace.full_pc_access {''',
    '''        agent_context.project.name,
        model.name,
        if agent_context.workspace.full_pc_access {''',
)

# Add focused unit coverage for the resume status boundary without needing a
# live model runtime.
test_anchor = '''    #[test]
    fn transcript_drops_old_context() {'''
new_test = '''    #[test]
    fn only_interrupted_runs_are_resumable() {
        assert!(is_resumable_run_status("interrupted"));
        assert!(!is_resumable_run_status("running"));
        assert!(!is_resumable_run_status("completed"));
        assert!(!is_resumable_run_status("failed"));
        assert!(!is_resumable_run_status("cancelled"));
    }

''' + test_anchor
replace_exact("src-tauri/src/local_agent.rs", test_anchor, new_test)

# Centralize the status check so command behavior and regression coverage stay
# aligned.
replace_exact(
    "src-tauri/src/local_agent.rs",
    '''        if run.status != "interrupted" {
            return Err(AppError::internal(
                "only an interrupted OpenAgent run can be resumed",
            ));
        }''',
    '''        if !is_resumable_run_status(&run.status) {
            return Err(AppError::internal(
                "only an interrupted OpenAgent run can be resumed",
            ));
        }''',
)
helper_anchor = '''fn parse_checkpoint_snapshot(raw: &str) -> Result<CheckpointSnapshot, AppError> {'''
replace_exact(
    "src-tauri/src/local_agent.rs",
    helper_anchor,
    '''fn is_resumable_run_status(status: &str) -> bool {
    status == "interrupted"
}

''' + helper_anchor,
)

print("interrupted run resume integration applied")
