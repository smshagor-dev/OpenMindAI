ALTER TABLE studio3d_jobs
ADD COLUMN output_object_count INTEGER NOT NULL DEFAULT 0
CHECK(output_object_count BETWEEN 0 AND 1024);
