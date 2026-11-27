ALTER TABLE studio3d_jobs
ADD COLUMN request_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(request_json));
