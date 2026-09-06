CREATE TABLE openagent_runs (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  project_id TEXT NOT NULL,
  assistant_message_id TEXT NOT NULL,
  model_id TEXT NOT NULL,
  goal TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'failed', 'cancelled', 'interrupted')),
  max_steps INTEGER NOT NULL,
  current_step INTEGER NOT NULL DEFAULT 0,
  consecutive_failures INTEGER NOT NULL DEFAULT 0,
  validation_status TEXT NOT NULL DEFAULT 'not_required'
    CHECK(validation_status IN ('not_required', 'required', 'passed', 'skipped')),
  validation_command TEXT,
  error TEXT,
  started_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT,
  FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
  FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
  FOREIGN KEY(assistant_message_id) REFERENCES messages(id) ON DELETE CASCADE,
  FOREIGN KEY(model_id) REFERENCES model_registry(id)
);

CREATE TABLE openagent_steps (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  step_index INTEGER NOT NULL,
  tool TEXT NOT NULL,
  action_json TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('running', 'succeeded', 'failed', 'blocked')),
  workspace_changed INTEGER NOT NULL DEFAULT 0,
  validation_command TEXT,
  result_summary TEXT,
  error TEXT,
  started_at TEXT NOT NULL,
  completed_at TEXT,
  FOREIGN KEY(run_id) REFERENCES openagent_runs(id) ON DELETE CASCADE,
  UNIQUE(run_id, step_index)
);

CREATE TABLE openagent_checkpoints (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  step_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('before_mutation', 'after_mutation', 'validation')),
  workspace_snapshot_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY(run_id) REFERENCES openagent_runs(id) ON DELETE CASCADE,
  FOREIGN KEY(step_id) REFERENCES openagent_steps(id) ON DELETE CASCADE
);

CREATE INDEX idx_openagent_runs_conversation_started
  ON openagent_runs(conversation_id, started_at DESC);
CREATE INDEX idx_openagent_steps_run_index
  ON openagent_steps(run_id, step_index ASC);
CREATE INDEX idx_openagent_checkpoints_run_created
  ON openagent_checkpoints(run_id, created_at ASC);
