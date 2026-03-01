BEGIN;

ALTER TABLE swissrpg_event
ADD CONSTRAINT swissrpg_event_url_key UNIQUE (url);

COMMIT;
