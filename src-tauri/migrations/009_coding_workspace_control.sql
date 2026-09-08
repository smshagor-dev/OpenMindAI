CREATE TABLE coding_run_plans (
  run_id TEXT PRIMARY KEY,
  revision INTEGER NOT NULL DEFAULT 1,
  plan_json TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY(run_id) REFERENCES openagent_runs(id) ON DELETE CASCADE
);

CREATE TABLE coding_run_events (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  sequence INTEGER NOT NULL,
  kind TEXT NOT NULL,
  label TEXT NOT NULL,
  detail_json TEXT,
  created_at TEXT NOT NULL,
  FOREIGN KEY(run_id) REFERENCES openagent_runs(id) ON DELETE CASCADE,
  UNIQUE(run_id, sequence)
);

CREATE INDEX idx_coding_run_events_run_sequence
  ON coding_run_events(run_id, sequence ASC);

CREATE TABLE coding_run_approvals (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  step_id TEXT,
  action_hash TEXT NOT NULL,
  tool TEXT NOT NULL,
  action_json TEXT NOT NULL,
  reason TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('pending', 'approved', 'rejected', 'consumed')),
  requested_at TEXT NOT NULL,
  decided_at TEXT,
  consumed_at TEXT,
  FOREIGN KEY(run_id) REFERENCES openagent_runs(id) ON DELETE CASCADE,
  FOREIGN KEY(step_id) REFERENCES openagent_steps(id) ON DELETE SET NULL
);

CREATE INDEX idx_coding_run_approvals_run_status
  ON coding_run_approvals(run_id, status, requested_at DESC);
CREATE INDEX idx_coding_run_approvals_hash_status
  ON coding_run_approvals(action_hash, status);

CREATE TABLE coding_run_links (
  parent_run_id TEXT NOT NULL,
  child_run_id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL,
  FOREIGN KEY(parent_run_id) REFERENCES openagent_runs(id) ON DELETE CASCADE,
  FOREIGN KEY(child_run_id) REFERENCES openagent_runs(id) ON DELETE CASCADE
);

CREATE TABLE coding_run_metrics (
  run_id TEXT PRIMARY KEY,
  prompt_tokens INTEGER NOT NULL DEFAULT 0,
  completion_tokens INTEGER NOT NULL DEFAULT 0,
  model_ms INTEGER NOT NULL DEFAULT 0,
  tool_ms INTEGER NOT NULL DEFAULT 0,
  tool_calls INTEGER NOT NULL DEFAULT 0,
  worker_calls INTEGER NOT NULL DEFAULT 0,
  local_cost_micros INTEGER NOT NULL DEFAULT 0,
  hardware_json TEXT NOT NULL DEFAULT '{}',
  budget_stop_reason TEXT,
  updated_at TEXT NOT NULL,
  FOREIGN KEY(run_id) REFERENCES openagent_runs(id) ON DELETE CASCADE
);

CREATE TABLE coding_eval_runs (
  id TEXT PRIMARY KEY,
  suite TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('running', 'passed', 'failed')),
  model_id TEXT,
  report_json TEXT NOT NULL,
  started_at TEXT NOT NULL,
  completed_at TEXT
);

CREATE INDEX idx_coding_eval_runs_started
  ON coding_eval_runs(started_at DESC);
