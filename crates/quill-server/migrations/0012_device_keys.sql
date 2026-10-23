-- Phase 2b: each persona's identity key and its certified devices.
--
-- Public keys only: private keys never leave clients. There are
-- deliberately no timestamps, and device ids are random, so devices that
-- were certified together for different personas can't be matched up.

-- A persona's identity key (Ed25519) and its version, set by its first
-- client. Unique across all personas: one key on two personas would link
-- them publicly.
ALTER TABLE actors
    ADD COLUMN identity_key bytea UNIQUE CHECK (octet_length(identity_key) = 32),
    ADD COLUMN identity_key_version integer CHECK (identity_key_version > 0),
    ADD CONSTRAINT actors_identity_key_pair
        CHECK ((identity_key IS NULL) = (identity_key_version IS NULL));

CREATE TABLE devices (
    -- Random (v4), never time-ordered.
    id          uuid    PRIMARY KEY,
    actor_id    uuid    NOT NULL REFERENCES actors (id) ON DELETE CASCADE,
    -- Unique across all personas, for the same reason as identity keys.
    device_key  bytea   NOT NULL UNIQUE CHECK (octet_length(device_key) = 32),
    -- The identity key version its certificate is signed with.
    key_version integer NOT NULL CHECK (key_version > 0),
    limited     boolean NOT NULL,
    certificate bytea   NOT NULL,
    -- Set once, never cleared: revocation is permanent.
    revocation  bytea
);

CREATE INDEX devices_actor_idx ON devices (actor_id);

-- A revocation can't be undone or replaced.
CREATE FUNCTION devices_revocation_final() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.revocation IS NOT NULL
       AND NEW.revocation IS DISTINCT FROM OLD.revocation THEN
        RAISE EXCEPTION 'a device revocation is permanent';
    END IF;
    IF OLD.revocation IS NOT NULL
       AND (NEW.certificate, NEW.key_version) IS DISTINCT FROM (OLD.certificate, OLD.key_version) THEN
        RAISE EXCEPTION 'a revoked device cannot be certified again';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER devices_revocation_final
    BEFORE UPDATE ON devices
    FOR EACH ROW EXECUTE FUNCTION devices_revocation_final();
