use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::{
    app_error::AppError,
    database::Database,
    openagent_runs::{OpenAgentRunDetails as LegacyRunDetails, OpenAgentRunRepository},
    AppState,
};

const MAX_PLAN_STEPS: usize = 12;
const MAX_PLAN_STEP_CHARS: usize = 500;
const MAX_EVENT_LABEL_CHARS: usize = 600;
const MAX_EVENT_DETAIL_CHARS: usize = 12_000;
const MAX_RESUME_SEED_CHARS: usize = 16_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingPlanStep {
    pub id: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingPlan {
    pub revision: i64,
    pub reason: String,
    pub steps: Vec<CodingPlanStep>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingEvent {
    pub id: String,
    pub run_id: String,
    pub sequence: i64,
    pub kind: String,
    pub label: String,
    pub detail_json: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingApproval {
    pub id: String,
    pub run_id: String,
    pub step_id: Option<String>,
    pub action_hash: String,
    pub tool: String,
    pub action_json: String,
    pub reason: String,
    pub status: String,
    pub requested_at: String,
    pub decided_at: Option<String>,
    pub consumed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodingMetrics {
    pub run_id: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub model_ms: i64,
    pub tool_ms: i64,
    pub tool_calls: i64,
    pub worker_calls: i64,
    pub local_cost_micros: i64,
    pub hardware_json: String,
    pub budget_stop_reason: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingRunSnapshot {
    pub details: LegacyRunDetails,
    pub plan: Option<CodingPlan>,
    pub events: Vec<CodingEvent>,
    pub approvals: Vec<CodingApproval>,
    pub metrics: Option<CodingMetrics>,
    pub parent_run_id: Option<String>,
}

pub fn initialize_run(
    database: &Database,
    run_id: &str,
    goal: &str,
    hardware_json: &str,
) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    let plan = CodingPlan {
        revision: 1,
        reason: "initial host plan".to_string(),
        steps: vec![
            plan_step("inspect", "Inspect repository instructions, architecture, symbols, tests, and current Git state", "active"),
            plan_step("change", "Make the smallest coherent workspace change using atomic edits", "pending"),
            plan_step("validate", "Run the strongest relevant local validation available", "pending"),
            plan_step("review", "Review diff, security boundaries, and regression risk", "pending"),
            plan_step("deliver", "Prepare controlled branch, commit, pull request, checks, repair, and merge when requested", "pending"),
        ],
    };
    database.connection().execute(
        "INSERT OR IGNORE INTO coding_run_plans (run_id, revision, plan_json, updated_at)
         VALUES (?1, 1, ?2, ?3)",
        params![run_id, serde_json::to_string(&plan)?, now],
    )?;
    database.connection().execute(
        "INSERT OR IGNORE INTO coding_run_metrics
         (run_id, hardware_json, updated_at) VALUES (?1, ?2, ?3)",
        params![run_id, hardware_json, now],
    )?;
    record_event(
        database,
        run_id,
        "run",
        "Coding run initialized",
        Some(&json!({"goal": bounded(goal, 2_000)})),
    )?;
    Ok(())
}

fn plan_step(id: &str, title: &str, status: &str) -> CodingPlanStep {
    CodingPlanStep {
        id: id.to_string(),
        title: title.to_string(),
        status: status.to_string(),
    }
}

pub fn load_plan(database: &Database, run_id: &str) -> Result<Option<CodingPlan>, AppError> {
    let value: Option<String> = database
        .connection()
        .query_row(
            "SELECT plan_json FROM coding_run_plans WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|raw| serde_json::from_str(&raw).map_err(|error| AppError::internal(error.to_string())))
        .transpose()
}

pub fn plan_text(database: &Database, run_id: &str) -> Result<String, AppError> {
    let Some(plan) = load_plan(database, run_id)? else {
        return Ok("No persisted plan is available.".to_string());
    };
    let mut lines = vec![format!("Plan revision {} — {}", plan.revision, plan.reason)];
    for (index, step) in plan.steps.iter().enumerate() {
        lines.push(format!("{}. [{}] {}", index + 1, step.status, step.title));
    }
    Ok(lines.join("\n"))
}

pub fn update_plan(
    database: &Database,
    run_id: &str,
    raw_steps: &[String],
    reason: &str,
) -> Result<CodingPlan, AppError> {
    let steps = raw_steps
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .take(MAX_PLAN_STEPS)
        .enumerate()
        .map(|(index, value)| CodingPlanStep {
            id: format!("step-{}", index + 1),
            title: bounded(value, MAX_PLAN_STEP_CHARS),
            status: if index == 0 { "active" } else { "pending" }.to_string(),
        })
        .collect::<Vec<_>>();
    if steps.is_empty() {
        return Err(AppError::internal("coding plan cannot be empty"));
    }
    let current = load_plan(database, run_id)?.map(|plan| plan.revision).unwrap_or(0);
    let plan = CodingPlan {
        revision: current.saturating_add(1),
        reason: bounded(reason, 1_000),
        steps,
    };
    database.connection().execute(
        "INSERT INTO coding_run_plans (run_id, revision, plan_json, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(run_id) DO UPDATE SET revision = excluded.revision,
           plan_json = excluded.plan_json, updated_at = excluded.updated_at",
        params![
            run_id,
            plan.revision,
            serde_json::to_string(&plan)?,
            Utc::now().to_rfc3339()
        ],
    )?;
    record_event(
        database,
        run_id,
        "plan",
        &format!("Plan revised to revision {}", plan.revision),
        Some(&json!({"reason": plan.reason, "steps": plan.steps})),
    )?;
    Ok(plan)
}

pub fn replan_after_failure(
    database: &Database,
    run_id: &str,
    failure: &str,
) -> Result<(), AppError> {
    let Some(current) = load_plan(database, run_id)? else {
        return Ok(());
    };
    let mut steps = vec![format!(
        "Diagnose the latest failure without repeating the same action: {}",
        bounded(failure, 500)
    )];
    steps.extend(current.steps.into_iter().map(|step| step.title));
    steps.truncate(MAX_PLAN_STEPS);
    update_plan(database, run_id, &steps, "automatic recovery replan")?;
    Ok(())
}

pub fn record_event(
    database: &Database,
    run_id: &str,
    kind: &str,
    label: &str,
    detail: Option<&Value>,
) -> Result<(), AppError> {
    let detail_json = detail.map(|value| bounded(&value.to_string(), MAX_EVENT_DETAIL_CHARS));
    database.connection().execute(
        "INSERT INTO coding_run_events (id, run_id, sequence, kind, label, detail_json, created_at)
         SELECT ?1, ?2, COALESCE(MAX(sequence), 0) + 1, ?3, ?4, ?5, ?6
         FROM coding_run_events WHERE run_id = ?2",
        params![
            Uuid::new_v4().to_string(),
            run_id,
            bounded(kind, 80),
            bounded(label, MAX_EVENT_LABEL_CHARS),
            detail_json,
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

pub fn request_approval(
    database: &Database,
    run_id: &str,
    step_id: Option<&str>,
    tool: &str,
    action_json: &str,
    reason: &str,
) -> Result<CodingApproval, AppError> {
    let hash = action_hash(tool, action_json);
    if let Some(existing) = find_approval(database, run_id, &hash, &["pending", "approved"])? {
        return Ok(existing);
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    database.connection().execute(
        "INSERT INTO coding_run_approvals
         (id, run_id, step_id, action_hash, tool, action_json, reason, status, requested_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8)",
        params![id, run_id, step_id, hash, tool, action_json, bounded(reason, 2_000), now],
    )?;
    record_event(
        database,
        run_id,
        "approval",
        &format!("Approval required for {tool}"),
        Some(&json!({"approvalId": id, "reason": reason})),
    )?;
    find_approval(database, run_id, &hash, &["pending"])?
        .ok_or_else(|| AppError::internal("new approval request was not found"))
}

pub fn consume_exact_approval(
    database: &Database,
    run_id: &str,
    tool: &str,
    action_json: &str,
) -> Result<bool, AppError> {
    let hash = action_hash(tool, action_json);
    let approval_id: Option<String> = database
        .connection()
        .query_row(
            "WITH RECURSIVE lineage(id) AS (
               SELECT ?1
               UNION ALL
               SELECT l.parent_run_id FROM coding_run_links l JOIN lineage x ON l.child_run_id = x.id
             )
             SELECT a.id FROM coding_run_approvals a
             JOIN lineage l ON l.id = a.run_id
             WHERE a.action_hash = ?2 AND a.status = 'approved'
             ORDER BY a.requested_at DESC LIMIT 1",
            params![run_id, hash],
            |row| row.get(0),
        )
        .optional()?;
    let Some(id) = approval_id else {
        return Ok(false);
    };
    let now = Utc::now().to_rfc3339();
    let changed = database.connection().execute(
        "UPDATE coding_run_approvals SET status = 'consumed', consumed_at = ?1
         WHERE id = ?2 AND status = 'approved'",
        params![now, id],
    )?;
    if changed == 1 {
        record_event(
            database,
            run_id,
            "approval",
            &format!("Consumed exact approval for {tool}"),
            Some(&json!({"actionHash": hash})),
        )?;
    }
    Ok(changed == 1)
}

fn find_approval(
    database: &Database,
    run_id: &str,
    hash: &str,
    statuses: &[&str],
) -> Result<Option<CodingApproval>, AppError> {
    let mut statement = database.connection().prepare(
        "SELECT id, run_id, step_id, action_hash, tool, action_json, reason, status,
                requested_at, decided_at, consumed_at
         FROM coding_run_approvals
         WHERE run_id = ?1 AND action_hash = ?2
         ORDER BY requested_at DESC",
    )?;
    let rows = statement.query_map(params![run_id, hash], map_approval)?;
    for row in rows {
        let approval = row?;
        if statuses.iter().any(|status| *status == approval.status) {
            return Ok(Some(approval));
        }
    }
    Ok(None)
}

fn action_hash(tool: &str, action_json: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(tool.as_bytes());
    digest.update([0]);
    digest.update(action_json.as_bytes());
    format!("{:x}", digest.finalize())
}

pub fn link_runs(database: &Database, parent: &str, child: &str) -> Result<(), AppError> {
    database.connection().execute(
        "INSERT OR REPLACE INTO coding_run_links (parent_run_id, child_run_id, created_at)
         VALUES (?1, ?2, ?3)",
        params![parent, child, Utc::now().to_rfc3339()],
    )?;
    record_event(
        database,
        child,
        "resume",
        "Continuation linked to interrupted run",
        Some(&json!({"parentRunId": parent})),
    )?;
    Ok(())
}

pub fn resume_seed(database: &Database, parent_run_id: &str) -> Result<String, AppError> {
    let run = OpenAgentRunRepository::new(database)
        .find(parent_run_id)?
        .ok_or_else(|| AppError::internal("interrupted coding run not found"))?;
    let details = OpenAgentRunRepository::new(database)
        .details(parent_run_id)?
        .ok_or_else(|| AppError::internal("interrupted coding run details not found"))?;
    let mut lines = vec![
        format!("CONTINUATION OF RUN {parent_run_id}"),
        format!("Original goal: {}", bounded(&run.goal, 2_000)),
        "Do not replay a mutation that already succeeded. Inspect current workspace state before changing anything.".to_string(),
    ];
    if let Some(plan) = load_plan(database, parent_run_id)? {
        lines.push(format!("Previous plan revision {}: {}", plan.revision, plan.reason));
        for step in plan.steps {
            lines.push(format!("- [{}] {}", step.status, step.title));
        }
    }
    for step in details.steps.iter().rev().take(12).rev() {
        let summary = step
            .result_summary
            .as_deref()
            .or(step.error.as_deref())
            .unwrap_or("no summary");
        lines.push(format!(
            "Previous step {} [{}] {}: {}",
            step.step_index,
            step.status,
            step.tool,
            bounded(summary, 800)
        ));
    }
    Ok(bounded(&lines.join("\n"), MAX_RESUME_SEED_CHARS))
}

pub fn estimate_tokens(text: &str) -> i64 {
    ((text.chars().count() as i64 + 3) / 4).max(1)
}

pub fn record_model_usage(
    database: &Database,
    run_id: &str,
    prompt_tokens: i64,
    completion_tokens: i64,
    elapsed_ms: u128,
    worker: bool,
) -> Result<(), AppError> {
    database.connection().execute(
        "UPDATE coding_run_metrics SET
           prompt_tokens = prompt_tokens + ?1,
           completion_tokens = completion_tokens + ?2,
           model_ms = model_ms + ?3,
           worker_calls = worker_calls + ?4,
           updated_at = ?5
         WHERE run_id = ?6",
        params![
            prompt_tokens.max(0),
            completion_tokens.max(0),
            i64::try_from(elapsed_ms).unwrap_or(i64::MAX),
            i64::from(worker),
            Utc::now().to_rfc3339(),
            run_id
        ],
    )?;
    Ok(())
}

pub fn record_tool_usage(
    database: &Database,
    run_id: &str,
    elapsed_ms: u128,
) -> Result<(), AppError> {
    database.connection().execute(
        "UPDATE coding_run_metrics SET tool_ms = tool_ms + ?1,
           tool_calls = tool_calls + 1, updated_at = ?2 WHERE run_id = ?3",
        params![
            i64::try_from(elapsed_ms).unwrap_or(i64::MAX),
            Utc::now().to_rfc3339(),
            run_id
        ],
    )?;
    Ok(())
}

pub fn budget_exceeded(
    database: &Database,
    run_id: &str,
    token_budget: i64,
    runtime_budget_minutes: i64,
) -> Result<Option<String>, AppError> {
    let metrics = load_metrics(database, run_id)?;
    if let Some(metrics) = metrics {
        let used = metrics.prompt_tokens.saturating_add(metrics.completion_tokens);
        if token_budget > 0 && used >= token_budget {
            return Ok(Some(format!("token budget exhausted ({used}/{token_budget})")));
        }
    }
    let started_at: Option<String> = database
        .connection()
        .query_row(
            "SELECT started_at FROM openagent_runs WHERE id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .optional()?;
    if runtime_budget_minutes > 0 {
        if let Some(started_at) = started_at {
            if let Ok(started) = DateTime::parse_from_rfc3339(&started_at) {
                let elapsed = Utc::now().signed_duration_since(started.with_timezone(&Utc));
                if elapsed.num_minutes() >= runtime_budget_minutes {
                    return Ok(Some(format!(
                        "runtime budget exhausted ({} minutes)",
                        runtime_budget_minutes
                    )));
                }
            }
        }
    }
    Ok(None)
}

pub fn set_budget_stop(database: &Database, run_id: &str, reason: &str) -> Result<(), AppError> {
    database.connection().execute(
        "UPDATE coding_run_metrics SET budget_stop_reason = ?1, updated_at = ?2 WHERE run_id = ?3",
        params![bounded(reason, 1_000), Utc::now().to_rfc3339(), run_id],
    )?;
    record_event(database, run_id, "budget", "Run stopped by budget controller", Some(&json!({"reason": reason})))
}

fn load_metrics(database: &Database, run_id: &str) -> Result<Option<CodingMetrics>, AppError> {
    database
        .connection()
        .query_row(
            "SELECT run_id, prompt_tokens, completion_tokens, model_ms, tool_ms, tool_calls,
                    worker_calls, local_cost_micros, hardware_json, budget_stop_reason, updated_at
             FROM coding_run_metrics WHERE run_id = ?1",
            params![run_id],
            |row| {
                Ok(CodingMetrics {
                    run_id: row.get(0)?,
                    prompt_tokens: row.get(1)?,
                    completion_tokens: row.get(2)?,
                    model_ms: row.get(3)?,
                    tool_ms: row.get(4)?,
                    tool_calls: row.get(5)?,
                    worker_calls: row.get(6)?,
                    local_cost_micros: row.get(7)?,
                    hardware_json: row.get(8)?,
                    budget_stop_reason: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(AppError::from)
}

fn load_events(database: &Database, run_id: &str) -> Result<Vec<CodingEvent>, AppError> {
    let mut statement = database.connection().prepare(
        "SELECT id, run_id, sequence, kind, label, detail_json, created_at
         FROM coding_run_events WHERE run_id = ?1 ORDER BY sequence ASC",
    )?;
    let rows = statement.query_map(params![run_id], |row| {
        Ok(CodingEvent {
            id: row.get(0)?,
            run_id: row.get(1)?,
            sequence: row.get(2)?,
            kind: row.get(3)?,
            label: row.get(4)?,
            detail_json: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

fn load_approvals(database: &Database, run_id: &str) -> Result<Vec<CodingApproval>, AppError> {
    let mut statement = database.connection().prepare(
        "SELECT id, run_id, step_id, action_hash, tool, action_json, reason, status,
                requested_at, decided_at, consumed_at
         FROM coding_run_approvals WHERE run_id = ?1 ORDER BY requested_at ASC",
    )?;
    let rows = statement.query_map(params![run_id], map_approval)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

fn map_approval(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodingApproval> {
    Ok(CodingApproval {
        id: row.get(0)?,
        run_id: row.get(1)?,
        step_id: row.get(2)?,
        action_hash: row.get(3)?,
        tool: row.get(4)?,
        action_json: row.get(5)?,
        reason: row.get(6)?,
        status: row.get(7)?,
        requested_at: row.get(8)?,
        decided_at: row.get(9)?,
        consumed_at: row.get(10)?,
    })
}

fn parent_run(database: &Database, run_id: &str) -> Result<Option<String>, AppError> {
    database
        .connection()
        .query_row(
            "SELECT parent_run_id FROM coding_run_links WHERE child_run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(AppError::from)
}

fn decide_approval(
    database: &Database,
    approval_id: &str,
    decision: &str,
) -> Result<CodingApproval, AppError> {
    if !matches!(decision, "approved" | "rejected") {
        return Err(AppError::internal("invalid approval decision"));
    }
    let now = Utc::now().to_rfc3339();
    let changed = database.connection().execute(
        "UPDATE coding_run_approvals SET status = ?1, decided_at = ?2
         WHERE id = ?3 AND status = 'pending'",
        params![decision, now, approval_id],
    )?;
    if changed != 1 {
        return Err(AppError::internal("approval request is no longer pending"));
    }
    let approval = database.connection().query_row(
        "SELECT id, run_id, step_id, action_hash, tool, action_json, reason, status,
                requested_at, decided_at, consumed_at
         FROM coding_run_approvals WHERE id = ?1",
        params![approval_id],
        map_approval,
    )?;
    record_event(
        database,
        &approval.run_id,
        "approval",
        &format!("{} action {}", approval.tool, decision),
        Some(&json!({"approvalId": approval.id})),
    )?;
    Ok(approval)
}

#[tauri::command]
pub fn coding_run_snapshot(
    run_id: String,
    state: State<AppState>,
) -> Result<Option<CodingRunSnapshot>, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let Some(details) = OpenAgentRunRepository::new(&db).details(&run_id)? else {
        return Ok(None);
    };
    Ok(Some(CodingRunSnapshot {
        details,
        plan: load_plan(&db, &run_id)?,
        events: load_events(&db, &run_id)?,
        approvals: load_approvals(&db, &run_id)?,
        metrics: load_metrics(&db, &run_id)?,
        parent_run_id: parent_run(&db, &run_id)?,
    }))
}

#[tauri::command]
pub fn approve_coding_action(
    approval_id: String,
    state: State<AppState>,
) -> Result<CodingApproval, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    decide_approval(&db, &approval_id, "approved")
}

#[tauri::command]
pub fn reject_coding_action(
    approval_id: String,
    state: State<AppState>,
) -> Result<CodingApproval, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    decide_approval(&db, &approval_id, "rejected")
}

fn bounded(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = value.chars().take(max_chars).collect::<String>();
    out.push_str("…[truncated]");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openagent_runs::OpenAgentRunRepository;

    fn seed_run(database: &Database) -> String {
        let conversation = Uuid::new_v4().to_string();
        let project = Uuid::new_v4().to_string();
        let message = Uuid::new_v4().to_string();
        let model = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        database.connection().execute(
            "INSERT INTO projects (id, name, instructions, created_at, updated_at) VALUES (?1, 'test', '', ?2, ?2)",
            params![project, now],
        ).unwrap();
        database.connection().execute(
            "INSERT INTO conversations (id, title, mode, created_at, updated_at) VALUES (?1, 'test', 'chat', ?2, ?2)",
            params![conversation, now],
        ).unwrap();
        database.connection().execute(
            "INSERT INTO project_conversations (project_id, conversation_id, created_at) VALUES (?1, ?2, ?3)",
            params![project, conversation, now],
        ).unwrap();
        database.connection().execute(
            "INSERT INTO model_registry (id, name, path, format, quantization, capabilities_json, min_ram_bytes, min_vram_bytes, enabled, created_at, updated_at)
             VALUES (?1, 'test', 'test.gguf', 'gguf', 'Q4', '[]', 0, NULL, 1, ?2, ?2)",
            params![model, now],
        ).unwrap();
        database.connection().execute(
            "INSERT INTO messages (id, conversation_id, role, content, status, model_id, created_at, updated_at)
             VALUES (?1, ?2, 'assistant', '', 'complete', ?3, ?4, ?4)",
            params![message, conversation, model, now],
        ).unwrap();
        OpenAgentRunRepository::new(database)
            .start(&conversation, &project, &message, &model, "fix tests", 10)
            .unwrap()
            .id
    }

    #[test]
    fn exact_approval_is_consumed_once() {
        let database = Database::in_memory().unwrap();
        let run_id = seed_run(&database);
        initialize_run(&database, &run_id, "fix tests", "{}").unwrap();
        let approval = request_approval(
            &database,
            &run_id,
            None,
            "delete_path",
            "{\"path\":\"a.txt\"}",
            "destructive operation",
        )
        .unwrap();
        decide_approval(&database, &approval.id, "approved").unwrap();
        assert!(consume_exact_approval(
            &database,
            &run_id,
            "delete_path",
            "{\"path\":\"a.txt\"}"
        )
        .unwrap());
        assert!(!consume_exact_approval(
            &database,
            &run_id,
            "delete_path",
            "{\"path\":\"a.txt\"}"
        )
        .unwrap());
    }

    #[test]
    fn replan_is_bounded_and_versioned() {
        let database = Database::in_memory().unwrap();
        let run_id = seed_run(&database);
        initialize_run(&database, &run_id, "goal", "{}").unwrap();
        let steps = (0..20).map(|index| format!("step {index}")).collect::<Vec<_>>();
        let plan = update_plan(&database, &run_id, &steps, "test").unwrap();
        assert_eq!(plan.revision, 2);
        assert_eq!(plan.steps.len(), MAX_PLAN_STEPS);
    }
}
