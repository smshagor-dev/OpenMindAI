CREATE TABLE research_title_jobs (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL REFERENCES model_registry(id),
    provider TEXT NOT NULL CHECK(provider='local-llama.cpp'),
    status TEXT NOT NULL CHECK(status IN ('running','proposal','accepted','failed','cancelled','interrupted')),
    prompt_text TEXT NOT NULL,
    prompt_sha256 TEXT NOT NULL,
    raw_output TEXT,
    proposed_title TEXT,
    output_sha256 TEXT,
    model_path_sha256 TEXT NOT NULL,
    model_context_length INTEGER,
    runtime_version TEXT,
    error TEXT,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    accepted_at TEXT,
    accepted_title TEXT
);
CREATE INDEX research_title_jobs_project_idx ON research_title_jobs(project_id, started_at DESC);
