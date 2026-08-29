-- A conversation has exactly one project owner. Older builds allowed the same
-- chat to be linked to several projects while inference selected only one,
-- which made project context ambiguous. Keep the most recently inserted link
-- for legacy databases, then enforce the invariant permanently.
DELETE FROM project_conversations
WHERE rowid NOT IN (
  SELECT MAX(rowid)
  FROM project_conversations
  GROUP BY conversation_id
);

CREATE UNIQUE INDEX idx_project_conversations_conversation
  ON project_conversations(conversation_id);
