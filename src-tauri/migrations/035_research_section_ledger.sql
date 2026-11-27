-- Evidence-bound, section-wise manuscript generation (Research Studio M5).
-- Source classification is explicit user metadata: numeric facts are only
-- offered to the local model from documents classified as raw experiment or
-- derived analysis output.
CREATE TABLE research_document_classes (
    document_id TEXT PRIMARY KEY REFERENCES research_documents(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    source_class TEXT NOT NULL CHECK (source_class IN (
        'raw_experiment','derived_analysis','audit_interpretation',
        'external_reference','software_fixture','unverifiable_claim'
    )),
    classified_at TEXT NOT NULL
);
CREATE INDEX research_document_classes_project_idx ON research_document_classes(project_id);

CREATE TABLE research_section_drafts (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    section_key TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    status TEXT NOT NULL CHECK (status IN (
        'running','draft','approved','superseded','failed','cancelled',
        'interrupted','insufficient_evidence'
    )),
    origin TEXT NOT NULL CHECK (origin IN ('local-model','human-edit','system')),
    content TEXT,
    content_sha256 TEXT,
    raw_output TEXT,
    output_sha256 TEXT,
    validation_json TEXT NOT NULL DEFAULT '{}',
    error TEXT,
    provider TEXT CHECK (provider IS NULL OR provider='local-llama.cpp'),
    model_id TEXT,
    model_path_sha256 TEXT,
    model_context_length INTEGER,
    runtime_version TEXT,
    conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
    prompt_text TEXT,
    prompt_sha256 TEXT,
    evidence_fact_set_sha256 TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    latency_ms INTEGER,
    base_draft_id TEXT REFERENCES research_section_drafts(id) ON DELETE SET NULL,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    approved_at TEXT,
    UNIQUE (project_id, section_key, revision)
);
CREATE INDEX research_section_drafts_project_idx
    ON research_section_drafts(project_id, section_key, revision DESC);

CREATE TABLE research_claim_ledger (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    section_draft_id TEXT NOT NULL REFERENCES research_section_drafts(id) ON DELETE CASCADE,
    paragraph_index INTEGER NOT NULL,
    claim_text TEXT NOT NULL,
    rendered_token TEXT,
    fact_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    document_name TEXT NOT NULL,
    document_sha256 TEXT NOT NULL,
    source_class TEXT NOT NULL,
    locator TEXT NOT NULL,
    metric TEXT NOT NULL,
    value_text TEXT NOT NULL,
    method TEXT NOT NULL,
    parent_fact_ids_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL
);
CREATE INDEX research_claim_ledger_draft_idx ON research_claim_ledger(section_draft_id);

CREATE TABLE research_section_exports (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES research_projects(id) ON DELETE CASCADE,
    artifact_id TEXT NOT NULL,
    format TEXT NOT NULL CHECK (format IN ('pdf','docx')),
    content_sha256 TEXT NOT NULL,
    file_sha256 TEXT,
    ledger_sha256 TEXT NOT NULL,
    draft INTEGER NOT NULL,
    page_target INTEGER NOT NULL,
    actual_pages INTEGER,
    word_count INTEGER NOT NULL,
    page_target_status TEXT NOT NULL CHECK (page_target_status IN ('TARGET_MET','INSUFFICIENT_EVIDENCE','NOT_MEASURED')),
    created_at TEXT NOT NULL
);
