CREATE TABLE research_figure_assets (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    source_run_id TEXT REFERENCES research_runs(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    media_type TEXT NOT NULL CHECK (media_type IN ('image/svg+xml','image/png','image/jpeg')),
    size_bytes INTEGER NOT NULL CHECK (size_bytes BETWEEN 1 AND 5242880),
    content_blob BLOB NOT NULL,
    content_sha256 TEXT NOT NULL,
    width_px INTEGER NOT NULL CHECK (width_px BETWEEN 1 AND 4096),
    height_px INTEGER NOT NULL CHECK (height_px BETWEEN 1 AND 4096),
    caption TEXT NOT NULL,
    alt_text TEXT NOT NULL,
    provenance_json TEXT NOT NULL,
    imported_at TEXT NOT NULL,
    UNIQUE (project_id, content_sha256)
);
CREATE INDEX research_figure_assets_project_idx
    ON research_figure_assets(project_id, imported_at DESC);
