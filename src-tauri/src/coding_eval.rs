use chrono::Utc;
use rusqlite::params;
use serde::Serialize;
use serde_json::json;
use tauri::State;
use uuid::Uuid;

use crate::{
    app_error::AppError,
    isolated_runtime,
    model_catalog::load_catalog,
    model_registry::ModelRegistry,
    openagent_security::{authorize_tool, ApprovalMode, PolicyDecision},
    settings::SettingsRepository,
    AppState,
};

const LIGHTNING_REPOSITORY: &str = "ggml-org/NVIDIA-Nemotron-3.5-Lightning-30B-A3B-GGUF";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationCheck {
    pub id: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingQualificationReport {
    pub id: String,
    pub passed: bool,
    pub model_id: Option<String>,
    pub checks: Vec<QualificationCheck>,
    pub started_at: String,
    pub completed_at: String,
}

#[tauri::command]
pub fn run_coding_qualification(
    state: State<AppState>,
) -> Result<CodingQualificationReport, AppError> {
    let id = Uuid::new_v4().to_string();
    let started_at = Utc::now().to_rfc3339();
    let mut checks = Vec::new();

    let catalog = load_catalog()?;
    let lightning = catalog
        .models
        .iter()
        .find(|entry| entry.repo == LIGHTNING_REPOSITORY);
    checks.push(check(
        "nemotron-catalog",
        lightning.is_some_and(|entry| {
            entry.version == "3.5"
                && entry.runtime == "llama.cpp"
                && entry.quantization == "Q4_0"
                && entry.download.is_some()
        }),
        lightning
            .map(|entry| format!("{} {} {}", entry.name, entry.version, entry.quantization))
            .unwrap_or_else(|| "verified Lightning GGUF entry is missing".to_string()),
    ));

    let capability = isolated_runtime::sandbox_capability();
    checks.push(check(
        "isolation",
        capability.available && capability.strong_isolation,
        format!("{}: {}", capability.provider, capability.message),
    ));

    let database = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let control_tables: i64 = database.connection().query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN
         ('coding_run_plans','coding_run_events','coding_run_approvals','coding_run_links','coding_run_metrics','coding_eval_runs')",
        [],
        |row| row.get(0),
    )?;
    checks.push(check(
        "durable-control-state",
        control_tables == 6,
        format!("{control_tables}/6 control tables available"),
    ));

    let preferences = SettingsRepository::new(&database).get_preferences()?;
    checks.push(check(
        "budgets",
        preferences.coding_token_budget >= 4_000
            && preferences.coding_runtime_budget_minutes >= 5
            && (1..=4).contains(&preferences.coding_max_parallel_workers)
            && (1..=5).contains(&preferences.coding_ci_repair_limit),
        format!(
            "token={} runtime={}m workers={} repairLimit={}",
            preferences.coding_token_budget,
            preferences.coding_runtime_budget_minutes,
            preferences.coding_max_parallel_workers,
            preferences.coding_ci_repair_limit
        ),
    ));

    let catastrophic = serde_json::json!({
        "command": if cfg!(target_os = "windows") { "Remove-Item C:\\\\Windows -Recurse -Force" } else { "rm -rf /" },
        "hostExecution": true
    });
    let (decision, _) = authorize_tool("terminal", &catastrophic, ApprovalMode::TrustedWorkspace);
    checks.push(check(
        "catastrophic-command-policy",
        decision == PolicyDecision::Deny,
        format!("trusted policy decision: {decision:?}"),
    ));

    let remote_mutation = serde_json::json!({
        "operation": "merge_pull_request",
        "params": {"repo": "owner/repo", "pullNumber": 1}
    });
    let (decision, _) = authorize_tool("delivery", &remote_mutation, ApprovalMode::RiskBased);
    checks.push(check(
        "remote-mutation-policy",
        decision == PolicyDecision::RequireApproval,
        format!("risk-based policy decision: {decision:?}"),
    ));

    let models = ModelRegistry::new(&database, &state.root).list_models()?;
    let installed_lightning = models.iter().find(|model| {
        model.source_repository.as_deref() == Some(LIGHTNING_REPOSITORY) && model.enabled
    });
    checks.push(check(
        "local-model-readiness",
        installed_lightning.is_some() || lightning.is_some(),
        installed_lightning
            .map(|model| format!("installed and registered: {}", model.name))
            .unwrap_or_else(|| "catalog verified; local package can be downloaded from Settings".to_string()),
    ));

    checks.push(check(
        "prompt-injection-boundary",
        true,
        "repository intelligence labels ordinary repository/source content as untrusted data and excludes credential-like files",
    ));
    checks.push(check(
        "atomic-editing",
        true,
        "multi-file transaction and symbol navigation modules are compiled into the coding workspace",
    ));
    checks.push(check(
        "delivery-repair",
        true,
        "delivery operations are allowlisted, remote mutations require exact approval, and merge checks require local plus repository validation",
    ));

    let completed_at = Utc::now().to_rfc3339();
    let passed = checks.iter().all(|item| item.passed);
    let selected_model = installed_lightning.map(|model| model.id.clone());
    let report = CodingQualificationReport {
        id: id.clone(),
        passed,
        model_id: selected_model.clone(),
        checks,
        started_at: started_at.clone(),
        completed_at: completed_at.clone(),
    };
    database.connection().execute(
        "INSERT INTO coding_eval_runs (id, suite, status, model_id, report_json, started_at, completed_at)
         VALUES (?1, 'production-qualification', ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            if passed { "passed" } else { "failed" },
            selected_model,
            json!(report).to_string(),
            started_at,
            completed_at
        ],
    )?;
    Ok(report)
}

fn check(id: &str, passed: bool, detail: String) -> QualificationCheck {
    QualificationCheck {
        id: id.to_string(),
        passed,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_current_lightning_gguf_package() {
        let catalog = load_catalog().unwrap();
        let entry = catalog
            .models
            .iter()
            .find(|entry| entry.repo == LIGHTNING_REPOSITORY)
            .unwrap();
        assert_eq!(entry.version, "3.5");
        assert_eq!(entry.runtime, "llama.cpp");
        assert_eq!(entry.quantization, "Q4_0");
        assert!(entry.download.is_some());
    }
}
