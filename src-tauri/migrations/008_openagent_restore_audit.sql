CREATE TABLE openagent_restore_events (
  id TEXT PRIMARY KEY,
  checkpoint_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'rolled_back', 'rollback_failed')),
  restored_files INTEGER NOT NULL DEFAULT 0,
  restored_directories INTEGER NOT NULL DEFAULT 0,
  removed_paths INTEGER NOT NULL DEFAULT 0,
  error TEXT,
  started_at TEXT NOT NULL,
  completed_at TEXT,
  FOREIGN KEY(checkpoint_id) REFERENCES openagent_checkpoints(id) ON DELETE CASCADE,
  FOREIGN KEY(run_id) REFERENCES openagent_runs(id) ON DELETE CASCADE
);

CREATE INDEX idx_openagent_restore_events_run_started
  ON openagent_restore_events(run_id, started_at DESC);
