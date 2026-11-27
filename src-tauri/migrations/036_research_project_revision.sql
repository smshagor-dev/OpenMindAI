ALTER TABLE research_projects
    ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0);

CREATE TRIGGER research_projects_revision_bump
AFTER UPDATE OF title, prompt, output_language, experiment_requested,
    manuscript_target_words, manuscript_page_target ON research_projects
FOR EACH ROW
WHEN NEW.revision = OLD.revision
BEGIN
    UPDATE research_projects SET revision = OLD.revision + 1 WHERE id = NEW.id;
END;
