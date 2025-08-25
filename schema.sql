CREATE TABLE managed_channel (
    discord_id bigint PRIMARY KEY
);

CREATE TABLE event_series_text_channel (
    discord_id bigint PRIMARY KEY,
    expiration_time timestamp (0) with time zone,
    last_expiration_reminder_time timestamp (0) with time zone,
    snooze_until timestamp (0) with time zone,
    deletion_time timestamp (0) with time zone, -- scheduled Discord channel deletion time
    deleted timestamp (0) with time zone -- set when Discord deletion is confirmed
);

CREATE TABLE event_series_voice_channel (
    discord_id bigint PRIMARY KEY,
    deletion_time timestamp (0) with time zone, -- scheduled Discord channel deletion time
    deleted timestamp (0) with time zone -- set when Discord deletion is confirmed
);

CREATE TABLE event_series_role (
    discord_id bigint PRIMARY KEY,
    deletion_time timestamp (0) with time zone,
    deleted timestamp (0) with time zone
);

CREATE TABLE event_series_host_role (
    discord_id bigint PRIMARY KEY,
    deletion_time timestamp (0) with time zone,
    deleted timestamp (0) with time zone
);

-- More flexible than an enum
CREATE TABLE event_series_type (
    "type" text PRIMARY KEY
);
INSERT INTO event_series_type ("type") VALUES ('campaign'), ('adventure');

CREATE SEQUENCE event_series_id_seq START WITH 1000;
CREATE TABLE event_series (
    id integer PRIMARY KEY DEFAULT nextval('event_series_id_seq'),
    discord_text_channel_id bigint UNIQUE REFERENCES event_series_text_channel (discord_id),
    discord_voice_channel_id bigint UNIQUE REFERENCES event_series_voice_channel (discord_id),
    discord_role_id bigint UNIQUE REFERENCES event_series_role (discord_id),
    discord_host_role_id bigint UNIQUE REFERENCES event_series_host_role (discord_id),
    discord_category_id bigint,
    "type" text NOT NULL REFERENCES event_series_type ("type"),
    redis_series_id text UNIQUE
);
ALTER SEQUENCE event_series_id_seq OWNED BY event_series.id;

CREATE SEQUENCE event_id_seq START WITH 1000;
CREATE TABLE event (
    id integer PRIMARY KEY DEFAULT nextval('event_id_seq'),
    event_series_id integer NOT NULL REFERENCES event_series (id),
    start_time timestamp (0) with time zone NOT NULL,
    end_time timestamp (0) with time zone,
    title text NOT NULL,
    description text NOT NULL,
    is_online boolean NOT NULL DEFAULT FALSE,
    discord_category_id bigint,
    deleted timestamp (0) with time zone
);
ALTER SEQUENCE event_id_seq OWNED BY event.id;
CREATE INDEX event_start_time_idx ON event USING btree (start_time);

CREATE SEQUENCE meetup_event_id_seq START WITH 1000;
CREATE TABLE meetup_event (
    id integer PRIMARY KEY DEFAULT nextval('meetup_event_id_seq'),
    event_id integer NOT NULL REFERENCES event (id),
    meetup_id text UNIQUE NOT NULL,
    url text NOT NULL,
    urlname text NOT NULL
);
ALTER SEQUENCE meetup_event_id_seq OWNED BY meetup_event.id;

CREATE SEQUENCE swissrpg_event_id_seq START WITH 1000;
CREATE TABLE swissrpg_event (
    id integer PRIMARY KEY DEFAULT nextval('swissrpg_event_id_seq'),
    event_id integer NOT NULL REFERENCES event (id),
    swissrpg_id uuid UNIQUE NOT NULL,
    url text NOT NULL
);
ALTER SEQUENCE swissrpg_event_id_seq OWNED BY swissrpg_event.id;

CREATE SEQUENCE member_id_seq START WITH 1000;
CREATE TABLE "member" (
    id integer PRIMARY KEY DEFAULT nextval('member_id_seq'),
	meetup_id bigint UNIQUE,
	discord_id bigint UNIQUE,
    discord_nick text,
    meetup_oauth2_access_token text,
    meetup_oauth2_refresh_token text,
    meetup_oauth2_last_token_refresh_time timestamp (0) with time zone,
    CONSTRAINT is_identifiable CHECK (meetup_id IS NOT NULL or discord_id IS NOT NULL)
);
ALTER SEQUENCE member_id_seq OWNED BY "member".id;

CREATE TABLE event_series_removed_host (
    event_series_id integer NOT NULL REFERENCES event_series (id),
    member_id integer NOT NULL REFERENCES "member" (id),
    removal_time timestamp (0) with time zone NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE event_series_removed_user (
    event_series_id integer NOT NULL REFERENCES event_series (id),
    member_id integer NOT NULL REFERENCES "member" (id),
    removal_time timestamp (0) with time zone NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE event_host (
	event_id integer NOT NULL REFERENCES event (id),
	member_id integer NOT NULL REFERENCES "member" (id),
	CONSTRAINT event_hosts_pk PRIMARY KEY (event_id, member_id)
);
CREATE INDEX event_hosts_event_id_idx ON event_host USING btree (event_id);
CREATE INDEX event_hosts_member_id_idx ON event_host USING btree (member_id);

CREATE TABLE event_participant (
	event_id integer NOT NULL REFERENCES event (id),
	member_id integer NOT NULL REFERENCES "member" (id),
	CONSTRAINT event_participants_pk PRIMARY KEY (event_id, member_id)
);
CREATE INDEX event_participants_event_id_idx ON event_participant USING btree (event_id);
CREATE INDEX event_participants_member_id_idx ON event_participant USING btree (member_id);

CREATE TABLE organizer_token (
    id bool PRIMARY KEY DEFAULT TRUE,
    meetup_access_token text NOT NULL,
    meetup_refresh_token text,
    meetup_access_token_refresh_time timestamp (0) with time zone,
    CONSTRAINT onerow CHECK (id)
);

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