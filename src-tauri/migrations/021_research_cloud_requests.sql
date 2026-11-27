CREATE TABLE research_cloud_requests (
    task_id TEXT PRIMARY KEY REFERENCES research_tasks(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES research_runs(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    request_state TEXT NOT NULL CHECK(request_state IN ('prepared','submitted','completed')),
    response_text TEXT,
    usage_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
