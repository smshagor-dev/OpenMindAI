-- Index all conversation message bodies locally without duplicating their text
-- in the full-text index. SQLite's trigram tokenizer supports phrase/substring
-- matches, including words appearing in the middle of a larger word.
CREATE VIRTUAL TABLE chat_message_fts USING fts5(
  content,
  content='messages',
  content_rowid='rowid',
  tokenize='trigram'
);

-- Keep the external-content index current across streaming chunks,
-- regeneration, destructive branch edits and cascaded chat deletion.
CREATE TRIGGER chat_message_fts_insert AFTER INSERT ON messages BEGIN
  INSERT INTO chat_message_fts(rowid, content) VALUES (new.rowid, new.content);
END;

CREATE TRIGGER chat_message_fts_delete AFTER DELETE ON messages BEGIN
  INSERT INTO chat_message_fts(chat_message_fts, rowid, content)
  VALUES ('delete', old.rowid, old.content);
END;

CREATE TRIGGER chat_message_fts_update AFTER UPDATE OF content ON messages BEGIN
  INSERT INTO chat_message_fts(chat_message_fts, rowid, content)
  VALUES ('delete', old.rowid, old.content);
  INSERT INTO chat_message_fts(rowid, content) VALUES (new.rowid, new.content);
END;

-- Existing installations already have chat histories; rebuild once during
-- migration so older conversations are searchable without reading them all in
-- the React renderer. All statements are transactionally applied by Database.
INSERT INTO chat_message_fts(chat_message_fts) VALUES ('rebuild');
