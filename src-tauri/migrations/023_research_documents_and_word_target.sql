ALTER TABLE research_projects
    ADD COLUMN manuscript_target_words INTEGER NOT NULL DEFAULT 3500
    CHECK (manuscript_target_words BETWEEN 3000 AND 4000);

CREATE TABLE research_documents (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    media_type TEXT NOT NULL CHECK (media_type IN ('application/pdf','application/vnd.openxmlformats-officedocument.wordprocessingml.document','text/markdown')),
    size_bytes INTEGER NOT NULL CHECK (size_bytes BETWEEN 1 AND 26214400),
    content_blob BLOB NOT NULL,
    content_sha256 TEXT NOT NULL,
    extracted_text TEXT NOT NULL,
    extraction_metadata_json TEXT NOT NULL,
    imported_at TEXT NOT NULL,
    UNIQUE (project_id, content_sha256)
);
CREATE INDEX research_documents_project_idx ON research_documents(project_id, imported_at DESC);

CREATE TABLE research_document_chunks (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES research_documents(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    start_char INTEGER NOT NULL,
    end_char INTEGER NOT NULL,
    content TEXT NOT NULL,
    UNIQUE (document_id, chunk_index)
);
CREATE INDEX research_document_chunks_project_idx ON research_document_chunks(project_id, document_id, chunk_index);
CREATE VIRTUAL TABLE research_document_chunks_fts USING fts5(
    chunk_id UNINDEXED,
    project_id UNINDEXED,
    content,
    tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER research_document_chunks_ai AFTER INSERT ON research_document_chunks BEGIN
    INSERT INTO research_document_chunks_fts (chunk_id, project_id, content)
    VALUES (new.id, new.project_id, new.content);
END;
CREATE TRIGGER research_document_chunks_ad AFTER DELETE ON research_document_chunks BEGIN
    DELETE FROM research_document_chunks_fts WHERE chunk_id = old.id;
END;
