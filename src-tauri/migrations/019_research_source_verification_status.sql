-- Compatibility migration: Database::migrate adds verification_status only
-- when absent, because early local checkpoint builds applied migration 016
-- before this source field was added to that migration.
SELECT 1;
