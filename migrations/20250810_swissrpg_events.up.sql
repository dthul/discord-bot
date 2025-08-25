BEGIN;

CREATE SEQUENCE swissrpg_event_id_seq START WITH 1000;
CREATE TABLE swissrpg_event (
    id integer PRIMARY KEY DEFAULT nextval('swissrpg_event_id_seq'),
    event_id integer NOT NULL REFERENCES event (id),
    swissrpg_id uuid UNIQUE NOT NULL,
    url text NOT NULL
);
ALTER SEQUENCE swissrpg_event_id_seq OWNED BY swissrpg_event.id;

COMMIT;