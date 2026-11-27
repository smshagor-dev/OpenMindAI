ALTER TABLE research_projects ADD COLUMN experiment_requested INTEGER NOT NULL DEFAULT 0
    CHECK(experiment_requested IN (0,1));
