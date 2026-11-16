-- Keep the previous reply durable until its replacement completes. The link
-- survives interruption; failed/cancelled replacements never delete their source.
ALTER TABLE messages ADD COLUMN regenerates_message_id TEXT
  REFERENCES messages(id) ON DELETE SET NULL;
CREATE INDEX idx_messages_regenerates ON messages(regenerates_message_id)
  WHERE regenerates_message_id IS NOT NULL;
