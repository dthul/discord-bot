BEGIN;

ALTER TABLE swissrpg_event
DROP CONSTRAINT IF EXISTS swissrpg_event_url_key;

COMMIT;
