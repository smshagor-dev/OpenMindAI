ALTER TABLE research_projects
    ADD COLUMN manuscript_page_target INTEGER NOT NULL DEFAULT 10
    CHECK (manuscript_page_target IN (6,8,10,12,15,20,24,25) OR manuscript_page_target BETWEEN 1 AND 100);
