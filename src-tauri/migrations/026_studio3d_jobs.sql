CREATE TABLE studio3d_jobs (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES studio3d_projects(id) ON DELETE CASCADE,
    generation_mode TEXT NOT NULL CHECK(generation_mode IN ('text-to-3d','image-to-3d','scene-edit','architecture')),
    inference_route TEXT NOT NULL CHECK(inference_route IN ('local','cloud','auto')),
    model_id TEXT NOT NULL DEFAULT '',
    engine_id TEXT NOT NULL DEFAULT '',
    input_asset_ids_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(input_asset_ids_json)),
    current_stage TEXT NOT NULL DEFAULT 'queued' CHECK(length(current_stage) <= 80),
    status TEXT NOT NULL CHECK(status IN ('queued','running','completed','failed','interrupted','cancelled','awaiting_approval')),
    input_sha256 TEXT NOT NULL DEFAULT '' CHECK(input_sha256 = '' OR length(input_sha256) = 64),
    output_asset_id TEXT REFERENCES studio3d_assets(id) ON DELETE SET NULL,
    output_sha256 TEXT NOT NULL DEFAULT '' CHECK(output_sha256 = '' OR length(output_sha256) = 64),
    error TEXT NOT NULL DEFAULT '',
    recovery_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(recovery_json)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT
);
CREATE INDEX studio3d_jobs_project_updated_idx ON studio3d_jobs(project_id, updated_at DESC, id);
CREATE INDEX studio3d_jobs_status_updated_idx ON studio3d_jobs(status, updated_at);
