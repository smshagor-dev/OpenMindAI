CREATE TABLE research_runs (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL REFERENCES model_registry(id),
    status TEXT NOT NULL CHECK(status IN ('queued','running','paused','completed','failed','cancelled','interrupted')),
    error TEXT,
    started_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT
);

CREATE INDEX research_runs_project_idx ON research_runs(project_id, started_at DESC);
CREATE UNIQUE INDEX research_runs_one_active_idx ON research_runs((1))
    WHERE status IN ('queued','running','paused');

CREATE TABLE research_tasks (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES research_runs(id) ON DELETE CASCADE,
    agent TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending','queued','running','completed','failed','cancelled','interrupted')),
    attempt INTEGER NOT NULL DEFAULT 0,
    result TEXT,
    error TEXT,
    started_at TEXT,
    updated_at TEXT NOT NULL,
    completed_at TEXT,
    UNIQUE(run_id, agent)
);

CREATE TABLE research_task_dependencies (
    task_id TEXT NOT NULL REFERENCES research_tasks(id) ON DELETE CASCADE,
    depends_on_task_id TEXT NOT NULL REFERENCES research_tasks(id) ON DELETE CASCADE,
    PRIMARY KEY(task_id, depends_on_task_id),
    CHECK(task_id <> depends_on_task_id)
);

CREATE TABLE research_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES research_runs(id) ON DELETE CASCADE,
    task_id TEXT REFERENCES research_tasks(id) ON DELETE SET NULL,
    agent TEXT,
    type TEXT NOT NULL,
    status TEXT,
    completed_units INTEGER,
    total_units INTEGER,
    timestamp TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX research_events_run_idx ON research_events(run_id, event_id);

CREATE TABLE research_sources (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES research_runs(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    canonical_url TEXT NOT NULL,
    title TEXT NOT NULL,
    doi TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    verification_status TEXT NOT NULL DEFAULT 'unverified'
        CHECK(verification_status IN ('verified','mismatch','retracted','unverified')),
    discovered_at TEXT NOT NULL,
    UNIQUE(run_id, canonical_url)
);

CREATE TABLE research_evidence (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES research_runs(id) ON DELETE CASCADE,
    task_id TEXT REFERENCES research_tasks(id) ON DELETE SET NULL,
    source_id TEXT REFERENCES research_sources(id) ON DELETE SET NULL,
    claim TEXT NOT NULL,
    quotation TEXT,
    provenance_json TEXT NOT NULL DEFAULT '{}',
    verification_status TEXT NOT NULL DEFAULT 'unverified'
        CHECK(verification_status IN ('verified','unverified','unsupported','mismatch')),
    created_at TEXT NOT NULL
);

CREATE INDEX research_evidence_run_idx ON research_evidence(run_id, created_at);
