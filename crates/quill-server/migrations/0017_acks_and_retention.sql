-- Phase 2g: delivery acknowledgements and retention.
--
-- Each device says how far it has read a session's log. A message is
-- deleted once every current device of both personas has acknowledged it
-- and it is older than the grace period. Acknowledgements are never shown
-- to the other writer.

-- Sequence numbers must never repeat, even once old messages are deleted,
-- so each group counts its own.
ALTER TABLE mls_groups ADD COLUMN last_seq bigint NOT NULL DEFAULT 0 CHECK (last_seq >= 0);
UPDATE mls_groups g SET last_seq = coalesce(
    (SELECT max(seq) FROM group_messages m WHERE m.group_id = g.group_id), 0);

ALTER TABLE group_messages ADD COLUMN logged_at timestamptz NOT NULL DEFAULT now();

CREATE TABLE session_acks (
    group_id  bytea  NOT NULL REFERENCES sessions (group_id) ON DELETE CASCADE,
    device_id uuid   NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    seq       bigint NOT NULL CHECK (seq >= 0),
    PRIMARY KEY (group_id, device_id)
);
