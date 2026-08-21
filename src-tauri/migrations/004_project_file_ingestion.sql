ALTER TABLE project_files ADD COLUMN content_text TEXT;
ALTER TABLE project_files ADD COLUMN status TEXT NOT NULL DEFAULT 'tracked';
ALTER TABLE project_files ADD COLUMN error TEXT;
