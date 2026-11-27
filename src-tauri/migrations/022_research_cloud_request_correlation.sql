ALTER TABLE research_cloud_requests ADD COLUMN client_request_id TEXT;
ALTER TABLE research_cloud_requests ADD COLUMN upstream_request_id TEXT;
ALTER TABLE research_cloud_requests ADD COLUMN attempt_no INTEGER NOT NULL DEFAULT 0;
ALTER TABLE research_cloud_requests ADD COLUMN retry_authorized INTEGER NOT NULL DEFAULT 0 CHECK(retry_authorized IN (0,1));
ALTER TABLE research_cloud_requests ADD COLUMN response_error TEXT;

UPDATE research_cloud_requests
SET attempt_no = CASE WHEN request_state='prepared' THEN 0 ELSE 1 END,
    response_error = CASE WHEN request_state='submitted' THEN 'Outcome unknown; this request was recorded before response finalization.' ELSE NULL END;
