CREATE TABLE studio3d_assets (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES studio3d_projects(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('image','mesh')),
    original_name TEXT NOT NULL CHECK(length(original_name) BETWEEN 1 AND 255),
    media_type TEXT NOT NULL CHECK(media_type IN ('image/jpeg','image/png','image/webp','image/bmp','image/tiff','model/gltf-binary')),
    size_bytes INTEGER NOT NULL CHECK(size_bytes BETWEEN 1 AND 31457280),
    content_sha256 TEXT NOT NULL CHECK(length(content_sha256) = 64),
    width INTEGER,
    height INTEGER,
    content_blob BLOB NOT NULL,
    preview_blob BLOB,
    created_at TEXT NOT NULL,
    UNIQUE(project_id, content_sha256)
);
CREATE INDEX studio3d_assets_project_idx ON studio3d_assets(project_id, created_at DESC, id);
