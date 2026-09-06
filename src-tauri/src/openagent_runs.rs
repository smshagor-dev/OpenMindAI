use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::{app_error::AppError, database::Database, AppState};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAgentRun {
    pub id: String,
    pub conversation_id: String,
    pub project_id: String,
    pub assistant_message_id: String,
    pub model_id: String,
    pub goal: String,
    pub status: String,
    pub max_steps: i64,
    pub current_step: i64,
    pub consecutive_failures: i64,
    pub validation_status: String,
    pub validation_command: Option<String>,
    pub error: Option<String>,
    pub started_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAgentStep {
    pub id: String,
    pub run_id: String,
    pub step_index: i64,
    pub tool: String,
    pub action_json: String,
    pub status: String,
    pub workspace_changed: bool,
    pub validation_command: Option<String>,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAgentRunDetails {
    pub run: OpenAgentRun,
    pub steps: Vec<OpenAgentStep>,
}

pub struct OpenAgentRunRepository<'a> {
    database: &'a Database,
}

impl<'a> OpenAgentRunRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn start(
        &self,
        conversation_id: &str,
        project_id: &str,
        assistant_message_id: &str,
        model_id: &str,
        goal: &str,
        max_steps: usize,
    ) -> Result<OpenAgentRun, AppError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "INSERT INTO openagent_runs
             (id, conversation_id, project_id, assistant_message_id, model_id, goal, status,
              max_steps, started_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7, ?8, ?8)",
            params![id, conversation_id, project_id, assistant_message_id, model_id, goal, max_steps as i64, now],
        )?;
        self.find(&id)?.ok_or_else(|| AppError::internal("new OpenAgent run was not found"))
    }

    pub fn start_step(&self, run_id: &str, step_index: usize, tool: &str, action_json: &str) -> Result<String, AppError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "INSERT INTO openagent_steps
             (id, run_id, step_index, tool, action_json, status, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
            params![id, run_id, step_index as i64, tool, action_json, now],
        )?;
        self.database.connection().execute(
            "UPDATE openagent_runs SET current_step = ?1, updated_at = ?2 WHERE id = ?3",
            params![step_index as i64, now, run_id],
        )?;
        Ok(id)
    }

    pub fn finish_step(&self, step_id: &str, status: &str, changed: bool, validation_command: Option<&str>, result: Option<&str>, error: Option<&str>) -> Result<(), AppError> {
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "UPDATE openagent_steps SET status = ?1, workspace_changed = ?2,
             validation_command = ?3, result_summary = ?4, error = ?5, completed_at = ?6
             WHERE id = ?7",
            params![status, changed as i64, validation_command, result, error, now, step_id],
        )?;
        Ok(())
    }

    pub fn checkpoint(&self, run_id: &str, step_id: &str, kind: &str, snapshot_json: &str) -> Result<(), AppError> {
        self.database.connection().execute(
            "INSERT INTO openagent_checkpoints (id, run_id, step_id, kind, workspace_snapshot_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![Uuid::new_v4().to_string(), run_id, step_id, kind, snapshot_json, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn update_progress(&self, run_id: &str, failures: usize, validation_status: &str, validation_command: Option<&str>) -> Result<(), AppError> {
        self.database.connection().execute(
            "UPDATE openagent_runs SET consecutive_failures = ?1, validation_status = ?2,
             validation_command = ?3, updated_at = ?4 WHERE id = ?5",
            params![failures as i64, validation_status, validation_command, Utc::now().to_rfc3339(), run_id],
        )?;
        Ok(())
    }

    pub fn finish(&self, run_id: &str, status: &str, error: Option<&str>) -> Result<(), AppError> {
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "UPDATE openagent_runs SET status = ?1, error = ?2, updated_at = ?3, completed_at = ?3 WHERE id = ?4",
            params![status, error, now, run_id],
        )?;
        Ok(())
    }

    pub fn list(&self, conversation_id: &str, limit: usize) -> Result<Vec<OpenAgentRun>, AppError> {
        let mut statement = self.database.connection().prepare(
            "SELECT id, conversation_id, project_id, assistant_message_id, model_id, goal, status,
             max_steps, current_step, consecutive_failures, validation_status, validation_command,
             error, started_at, updated_at, completed_at FROM openagent_runs
             WHERE conversation_id = ?1 ORDER BY started_at DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![conversation_id, limit.min(100) as i64], map_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn find(&self, run_id: &str) -> Result<Option<OpenAgentRun>, AppError> {
        self.database.connection().query_row(
            "SELECT id, conversation_id, project_id, assistant_message_id, model_id, goal, status,
             max_steps, current_step, consecutive_failures, validation_status, validation_command,
             error, started_at, updated_at, completed_at FROM openagent_runs WHERE id = ?1",
            params![run_id], map_run,
        ).optional().map_err(AppError::from)
    }

    pub fn details(&self, run_id: &str) -> Result<Option<OpenAgentRunDetails>, AppError> {
        let Some(run) = self.find(run_id)? else { return Ok(None); };
        let mut statement = self.database.connection().prepare(
            "SELECT id, run_id, step_index, tool, action_json, status, workspace_changed,
             validation_command, result_summary, error, started_at, completed_at
             FROM openagent_steps WHERE run_id = ?1 ORDER BY step_index ASC",
        )?;
        let rows = statement.query_map(params![run_id], map_step)?;
        let steps = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(Some(OpenAgentRunDetails { run, steps }))
    }
}

fn map_run(row: &Row<'_>) -> rusqlite::Result<OpenAgentRun> {
    Ok(OpenAgentRun { id: row.get(0)?, conversation_id: row.get(1)?, project_id: row.get(2)?, assistant_message_id: row.get(3)?, model_id: row.get(4)?, goal: row.get(5)?, status: row.get(6)?, max_steps: row.get(7)?, current_step: row.get(8)?, consecutive_failures: row.get(9)?, validation_status: row.get(10)?, validation_command: row.get(11)?, error: row.get(12)?, started_at: row.get(13)?, updated_at: row.get(14)?, completed_at: row.get(15)? })
}

fn map_step(row: &Row<'_>) -> rusqlite::Result<OpenAgentStep> {
    Ok(OpenAgentStep { id: row.get(0)?, run_id: row.get(1)?, step_index: row.get(2)?, tool: row.get(3)?, action_json: row.get(4)?, status: row.get(5)?, workspace_changed: row.get::<_, i64>(6)? != 0, validation_command: row.get(7)?, result_summary: row.get(8)?, error: row.get(9)?, started_at: row.get(10)?, completed_at: row.get(11)? })
}

#[tauri::command]
pub fn list_openagent_runs(conversation_id: String, limit: Option<usize>, state: State<AppState>) -> Result<Vec<OpenAgentRun>, AppError> {
    let db = state.database.lock().map_err(|_| AppError::internal("database lock poisoned"))?;
    OpenAgentRunRepository::new(&db).list(&conversation_id, limit.unwrap_or(20))
}

#[tauri::command]
pub fn openagent_run_details(run_id: String, state: State<AppState>) -> Result<Option<OpenAgentRunDetails>, AppError> {
    let db = state.database.lock().map_err(|_| AppError::internal("database lock poisoned"))?;
    OpenAgentRunRepository::new(&db).details(&run_id)
}
