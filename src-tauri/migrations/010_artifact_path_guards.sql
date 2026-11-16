-- Historical duplicate rows remain readable; new collisions are rejected.
CREATE TRIGGER artifacts_unique_path_insert BEFORE INSERT ON artifacts
WHEN EXISTS(SELECT 1 FROM artifacts WHERE path = NEW.path)
BEGIN SELECT RAISE(ABORT, 'artifact output path already reserved'); END;
CREATE TRIGGER artifacts_unique_path_update BEFORE UPDATE OF path ON artifacts
WHEN NEW.path != OLD.path AND EXISTS(SELECT 1 FROM artifacts WHERE path = NEW.path)
BEGIN SELECT RAISE(ABORT, 'artifact output path already reserved'); END;
