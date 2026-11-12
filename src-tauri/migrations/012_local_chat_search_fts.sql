-- Local-only trigram index backed by the existing messages table.
-- Search indexes message bodies without duplicating their source text.
CREATE VIRTUAL TABLE chat_message_fts USING fts5(
  content,
  content='messages',
  content_rowid='rowid',
  tokenize='trigram'
);

-- Streaming assistant messages update on every token chunk. Indexing each
-- growing token prefix would re-tokenize the entire response O(n^2).
-- Only index stable (non-pending, non-streaming) non-system messages.
CREATE TRIGGER chat_message_fts_insert AFTER INSERT ON messages
WHEN new.role != 'system' AND new.status NOT IN ('pending', 'streaming') BEGIN
  INSERT INTO chat_message_fts(rowid, content) VALUES (new.rowid, new.content);
END;

CREATE TRIGGER chat_message_fts_delete AFTER DELETE ON messages
WHEN old.role != 'system' AND old.status NOT IN ('pending', 'streaming') BEGIN
  INSERT INTO chat_message_fts(chat_message_fts, rowid, content)
  VALUES ('delete', old.rowid, old.content);
END;

CREATE TRIGGER chat_message_fts_update AFTER UPDATE OF content, status, role ON messages BEGIN
  INSERT INTO chat_message_fts(chat_message_fts, rowid, content)
  SELECT 'delete', old.rowid, old.content
  WHERE old.role != 'system' AND old.status NOT IN ('pending', 'streaming');

  INSERT INTO chat_message_fts(rowid, content)
  SELECT new.rowid, new.content
  WHERE new.role != 'system' AND new.status NOT IN ('pending', 'streaming');
END;

-- Backfill existing profiles once, transactionally, without performing
-- a full-messages rebuild that would index system or in-flight text.
INSERT INTO chat_message_fts(rowid, content)
SELECT rowid, content FROM messages
WHERE role != 'system' AND status NOT IN ('pending', 'streaming');
