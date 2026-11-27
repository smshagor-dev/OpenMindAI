CREATE TABLE research_run_routing (
    run_id TEXT PRIMARY KEY REFERENCES research_runs(id) ON DELETE CASCADE,
    requested_mode TEXT NOT NULL CHECK(requested_mode IN ('local','cloud','auto')),
    selected_provider TEXT,
    selected_model TEXT,
    usage_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL
);
