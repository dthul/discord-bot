BEGIN;

-- Remove the index
DROP INDEX IF EXISTS event_series_swissrpg_event_series_id_idx;

-- Remove the unique constraint
ALTER TABLE event_series 
DROP CONSTRAINT IF EXISTS event_series_swissrpg_event_series_id_unique;

-- Remove the column
ALTER TABLE event_series 
DROP COLUMN IF EXISTS swissrpg_event_series_id;

COMMIT;