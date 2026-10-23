-- Phase 2f: sessions after acceptance.
--
-- A session is an accepted request's MLS group (kind 'session' in
-- mls_groups, its two personas in session_members). The server keeps the
-- request's Welcome for the devices that were in it, and Welcomes left for
-- devices added later. All of it is ciphertext.

CREATE TABLE sessions (
    group_id   bytea       PRIMARY KEY REFERENCES mls_groups (group_id) ON DELETE CASCADE,
    -- So clients can match the session to the request they sent or read.
    request_id uuid        NOT NULL UNIQUE,
    prompt_id  uuid        REFERENCES prompts (id) ON DELETE SET NULL,
    -- The request's Welcome, for the devices that were in it.
    welcome    bytea       NOT NULL,
    -- Set when a writer leaves (or a member persona is deleted). The log
    -- stays readable; nothing more is accepted for it.
    ended_at   timestamptz
);

-- Sessions accepted under 2e.
INSERT INTO sessions (group_id, request_id, prompt_id, welcome)
    SELECT group_id, id, prompt_id, welcome FROM session_requests
    WHERE state = 'accepted' AND welcome IS NOT NULL;
UPDATE session_requests SET welcome = NULL WHERE state = 'accepted';

-- Welcomes for devices added to a session after it began, one per device.
CREATE TABLE session_welcomes (
    group_id  bytea  NOT NULL REFERENCES sessions (group_id) ON DELETE CASCADE,
    device_id uuid   NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    welcome   bytea  NOT NULL,
    -- Where the device starts reading the log.
    from_seq  bigint NOT NULL CHECK (from_seq >= 0),
    PRIMARY KEY (group_id, device_id)
);

CREATE INDEX session_welcomes_device_idx ON session_welcomes (device_id);

-- A member persona deleted (with its login, or on its own): its sessions
-- end.
CREATE FUNCTION sessions_member_gone() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    UPDATE sessions SET ended_at = now()
    WHERE group_id = OLD.group_id AND ended_at IS NULL;
    RETURN OLD;
END;
$$;

CREATE TRIGGER sessions_member_gone
    AFTER DELETE ON session_members
    FOR EACH ROW EXECUTE FUNCTION sessions_member_gone();
