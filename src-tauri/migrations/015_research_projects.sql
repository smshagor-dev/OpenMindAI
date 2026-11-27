CREATE TABLE research_projects (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    prompt TEXT NOT NULL,
    output_language TEXT NOT NULL DEFAULT 'English',
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'running', 'paused', 'completed', 'failed', 'cancelled')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX research_projects_updated_idx
    ON research_projects(updated_at DESC, id DESC);
