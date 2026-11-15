-- Transactional tombstones survive crashes and late index publication. Keep them
-- until a future blob/reference collector can retire them safely.
CREATE TABLE deleted_media_indexes (message_id TEXT PRIMARY KEY NOT NULL);
CREATE TRIGGER queue_deleted_media_index AFTER DELETE ON messages
BEGIN
    INSERT OR IGNORE INTO deleted_media_indexes(message_id) VALUES(OLD.id);
END;
