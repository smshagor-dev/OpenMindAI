CREATE TABLE studio3d_projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 120),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX studio3d_projects_updated_idx ON studio3d_projects(updated_at DESC, id DESC);

CREATE TABLE studio3d_scenes (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES studio3d_projects(id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 160),
    prompt TEXT NOT NULL CHECK(length(prompt) <= 8000),
    mode TEXT NOT NULL CHECK(mode IN ('object','architecture','imported')),
    engine TEXT NOT NULL CHECK(length(engine) <= 120),
    scene_json TEXT NOT NULL,
    glb_blob BLOB NOT NULL,
    content_sha256 TEXT NOT NULL CHECK(length(content_sha256) = 64),
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX studio3d_scenes_project_updated_idx ON studio3d_scenes(project_id, updated_at DESC, id);

CREATE TABLE studio3d_scene_revisions (
    scene_id TEXT NOT NULL REFERENCES studio3d_scenes(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision > 0),
    scene_json TEXT NOT NULL,
    glb_blob BLOB NOT NULL,
    content_sha256 TEXT NOT NULL CHECK(length(content_sha256) = 64),
    created_at TEXT NOT NULL,
    PRIMARY KEY(scene_id, revision)
);
