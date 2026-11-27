CREATE TABLE research_manuscript_revisions (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES research_runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('proposal','accepted')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(run_id, revision)
);

CREATE TABLE research_approvals (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES research_runs(id) ON DELETE CASCADE,
    revision_id TEXT NOT NULL REFERENCES research_manuscript_revisions(id) ON DELETE CASCADE,
    content_hash TEXT NOT NULL,
    approval_type TEXT NOT NULL CHECK(approval_type IN ('manuscript_acceptance')),
    approved_at TEXT NOT NULL
);

CREATE INDEX research_approvals_revision_idx ON research_approvals(revision_id, approved_at DESC);
CREATE UNIQUE INDEX research_approvals_once_idx
    ON research_approvals(revision_id, content_hash, approval_type);

CREATE TABLE research_exports (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES research_runs(id) ON DELETE CASCADE,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    format TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    approved INTEGER NOT NULL CHECK(approved IN (0,1)),
    created_at TEXT NOT NULL
);
