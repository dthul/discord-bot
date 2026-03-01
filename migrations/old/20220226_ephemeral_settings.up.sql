BEGIN;

CREATE TABLE ephemeral_settings (
    id bool PRIMARY KEY DEFAULT TRUE,
    cookie_key bytea,
    CONSTRAINT onerow CHECK (id)
);

CREATE SEQUENCE web_session_id_seq START WITH 1000;
CREATE TABLE web_session (
    id integer PRIMARY KEY DEFAULT nextval('web_session_id_seq'),
    session_id bytea NOT NULL UNIQUE,
    member_id integer NOT NULL REFERENCES "member" (id),
    last_used timestamp (0) with time zone NOT NULL DEFAULT CURRENT_TIMESTAMP
);
ALTER SEQUENCE web_session_id_seq OWNED BY web_session.id;

COMMIT;