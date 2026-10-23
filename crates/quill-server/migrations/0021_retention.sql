-- Phase 2k: deleting by age, and history bundles in the device group.

-- How far each group's log has been deleted. Reading from before it gets
-- 410 Gone: the reader has missed commits and can't follow the group.
ALTER TABLE mls_groups ADD COLUMN pruned_through bigint NOT NULL DEFAULT 0;

-- The age sweep finds old messages by when they were logged.
CREATE INDEX group_messages_logged_idx ON group_messages (logged_at);

-- Blobs may now belong to a login's personal device group (history
-- bundles), not only to sessions. A session's group row outlives nothing
-- the session doesn't, so the cascade is unchanged for sessions.
ALTER TABLE blobs DROP CONSTRAINT blobs_group_id_fkey;
ALTER TABLE blobs ADD CONSTRAINT blobs_group_id_fkey
    FOREIGN KEY (group_id) REFERENCES mls_groups (group_id) ON DELETE CASCADE;
