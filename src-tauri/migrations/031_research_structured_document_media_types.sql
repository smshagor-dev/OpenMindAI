DROP TRIGGER research_document_chunks_ai;
DROP TRIGGER research_document_chunks_ad;
DROP TABLE research_document_chunks_fts;
CREATE TEMP TABLE research_document_chunks_backup AS
    SELECT id,document_id,project_id,chunk_index,start_char,end_char,content FROM research_document_chunks;
DROP TABLE research_document_chunks;

CREATE TABLE research_documents_replacement (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    media_type TEXT NOT NULL CHECK (media_type IN (
        'application/pdf',
        'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
        'text/markdown',
        'text/csv',
        'application/json',
        'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'
    )),
    size_bytes INTEGER NOT NULL CHECK (size_bytes BETWEEN 1 AND 26214400),
    content_blob BLOB NOT NULL,
    content_sha256 TEXT NOT NULL,
    extracted_text TEXT NOT NULL,
    extraction_metadata_json TEXT NOT NULL,
    imported_at TEXT NOT NULL,
    UNIQUE (project_id, content_sha256)
);
INSERT INTO research_documents_replacement
SELECT id,project_id,name,media_type,size_bytes,content_blob,content_sha256,extracted_text,extraction_metadata_json,imported_at
FROM research_documents;
DROP TABLE research_documents;
ALTER TABLE research_documents_replacement RENAME TO research_documents;
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
INSERT INTO research_document_chunks SELECT id,document_id,project_id,chunk_index,start_char,end_char,content FROM research_document_chunks_backup;
DROP TABLE research_document_chunks_backup;
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
INSERT INTO research_document_chunks_fts(chunk_id,project_id,content)
SELECT id,project_id,content FROM research_document_chunks;
