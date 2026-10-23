-- Phase 1g: a deleted persona's handle is retired for good, so nobody can
-- take over a name that readers (and, later, other servers) know. Only the
-- handle is kept, not which account had it.

CREATE TABLE retired_handles (
    handle     text        PRIMARY KEY,
    retired_at timestamptz NOT NULL DEFAULT now()
);

-- A trigger, so every way a persona disappears is covered, including an
-- account's deletion cascading to its personas.
CREATE FUNCTION retire_handle() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO retired_handles (handle) VALUES (OLD.preferred_username)
    ON CONFLICT DO NOTHING;
    RETURN OLD;
END;
$$;

CREATE TRIGGER actors_retire_handle
    AFTER DELETE ON actors
    FOR EACH ROW EXECUTE FUNCTION retire_handle();

-- Retired handles can't be used again, whatever inserts the persona.
CREATE FUNCTION refuse_retired_handle() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM retired_handles WHERE handle = NEW.preferred_username) THEN
        RAISE EXCEPTION 'handle % is retired', NEW.preferred_username
            USING ERRCODE = 'unique_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER actors_refuse_retired_handle
    BEFORE INSERT ON actors
    FOR EACH ROW EXECUTE FUNCTION refuse_retired_handle();
