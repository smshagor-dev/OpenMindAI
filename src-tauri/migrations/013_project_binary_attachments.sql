-- Binary project attachments live inside the portable SQLite profile.
-- This keeps VACUUM INTO backups, file deletion and portable-root moves
-- atomic with their metadata, without trusting user filenames as OS paths.
ALTER TABLE project_files ADD COLUMN content_blob BLOB;
ALTER TABLE project_files ADD COLUMN content_sha256 TEXT;

-- Historic non-text uploads stored ONLY metadata. They cannot be restored.
-- Make the limitation explicit instead of implying the original still exists.
UPDATE project_files
SET error = 'Metadata only: the original file was not saved. Re-upload to attach its bytes.'
WHERE status = 'tracked' AND content_text IS NULL;
