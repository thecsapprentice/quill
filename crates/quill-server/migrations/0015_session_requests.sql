-- Phase 2e: collaboration requests, and session groups on the group
-- message log.
--
-- A request carries an MLS Welcome and the encrypted pitch; the server
-- reads only the pitch's clear header (group id, epoch). Accepting turns
-- the request's group into a session.

ALTER TABLE mls_groups DROP CONSTRAINT mls_groups_kind_check;
ALTER TABLE mls_groups ADD CONSTRAINT mls_groups_kind_check
    CHECK (kind IN ('personal', 'session'));

-- The personas taking part in a session group. Their logins can read and
-- post to its log.
CREATE TABLE session_members (
    group_id bytea NOT NULL REFERENCES mls_groups (group_id) ON DELETE CASCADE,
    actor_id uuid  NOT NULL REFERENCES actors (id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, actor_id)
);

CREATE INDEX session_members_actor_idx ON session_members (actor_id);

CREATE TYPE request_state AS ENUM (
    -- Waiting: the requester sees "waiting" for all four until expiry.
    'pending', 'held', 'refused', 'declined',
    'accepted', 'expired'
);

CREATE TABLE session_requests (
    -- Random (v4), never time-ordered.
    id                   uuid          PRIMARY KEY,
    -- Cleared if the prompt is deleted (its waiting requests expire first).
    prompt_id            uuid          REFERENCES prompts (id) ON DELETE SET NULL,
    author_actor_id      uuid          NOT NULL REFERENCES actors (id) ON DELETE CASCADE,
    requester_actor_id   uuid          NOT NULL REFERENCES actors (id) ON DELETE CASCADE,
    -- For the one-waiting-request-per-login rule. The server already knows
    -- which login each persona belongs to; this adds nothing to that.
    requester_account_id uuid          NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    group_id             bytea         NOT NULL UNIQUE
                                       CHECK (octet_length(group_id) BETWEEN 16 AND 64),
    -- The epoch the pitch was sent in: the session's log starts there.
    epoch                bigint        NOT NULL CHECK (epoch >= 0),
    state                request_state NOT NULL,
    -- Ciphertext, kept only while someone may still need it: dropped when a
    -- request is refused, declined or expires. The pitch moves into the
    -- session's log on acceptance; the Welcome stays, for 2f's devices.
    welcome              bytea,
    pitch                bytea,
    received_at          timestamptz   NOT NULL DEFAULT now(),
    expires_at           timestamptz   NOT NULL,
    CHECK (state IN ('pending', 'held', 'accepted') OR pitch IS NULL),
    CHECK (state IN ('pending', 'held', 'accepted') OR welcome IS NULL)
);

-- One waiting request per login per prompt, across its personas.
CREATE UNIQUE INDEX session_requests_one_waiting_idx
    ON session_requests (prompt_id, requester_account_id)
    WHERE state IN ('pending', 'held', 'refused', 'declined');

CREATE INDEX session_requests_queue_idx ON session_requests (prompt_id, state);
CREATE INDEX session_requests_requester_idx ON session_requests (requester_account_id);
CREATE INDEX session_requests_expiry_idx ON session_requests (expires_at)
    WHERE state IN ('pending', 'held', 'refused', 'declined');
