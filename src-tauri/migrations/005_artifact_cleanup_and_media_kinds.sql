CREATE TABLE artifacts_v2 (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  message_id TEXT,
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  mime_type TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('text', 'markdown', 'code', 'pdf', 'docx', 'image', 'audio', 'video')),
  size_bytes INTEGER NOT NULL DEFAULT 0,
  page_count INTEGER,
  status TEXT NOT NULL CHECK(status IN ('generating', 'ready', 'failed')),
  error TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
  FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE SET NULL
);

INSERT INTO artifacts_v2 (
  id, conversation_id, message_id, name, path, mime_type, kind, size_bytes,
  page_count, status, error, created_at, updated_at
)
SELECT
  id, conversation_id, message_id, name, path, mime_type, kind, size_bytes,
  page_count, status, error, created_at, updated_at
FROM artifacts;

DROP TABLE artifacts;
ALTER TABLE artifacts_v2 RENAME TO artifacts;

CREATE INDEX idx_artifacts_conversation ON artifacts(conversation_id);

CREATE TRIGGER delete_artifact_file_before_row
BEFORE DELETE ON artifacts
FOR EACH ROW
BEGIN
  SELECT openmind_delete_artifact(OLD.path);
END;
