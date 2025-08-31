BEGIN;

-- Add the swissrpg_event_series_id column to event_series table
ALTER TABLE event_series 
ADD COLUMN swissrpg_event_series_id uuid;

-- Add unique constraint (allows NULL values)
ALTER TABLE event_series 
ADD CONSTRAINT event_series_swissrpg_event_series_id_unique 
UNIQUE (swissrpg_event_series_id);

-- Create index for performance on lookups
CREATE INDEX event_series_swissrpg_event_series_id_idx 
ON event_series USING btree (swissrpg_event_series_id);

COMMIT;