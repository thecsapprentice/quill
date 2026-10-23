-- Phase 1e: tag moderation. Server settings stored in the database, hidden
-- tags, and tags awaiting an administrator's approval.

-- Settings administrators change while the server runs. Environment
-- variables only provide the starting values.
CREATE TABLE server_settings (
    key        text        PRIMARY KEY,
    value      text        NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- visible: shown to everyone.
-- hidden:  kept on prompts, shown only to administrators.
-- pending: a writer's new tag awaiting approval; shown only to
--          administrators and the authors of prompts that carry it.
ALTER TABLE tags
    ADD COLUMN status text NOT NULL DEFAULT 'visible'
        CHECK (status IN ('visible', 'hidden', 'pending'));

CREATE INDEX tags_pending_idx ON tags (created_at) WHERE status = 'pending';

ALTER TABLE admin_log
    DROP CONSTRAINT admin_log_action_check,
    ADD CONSTRAINT admin_log_action_check CHECK (action IN (
        'report_received', 'discard', 'delete_prompt',
        'create_tag_type', 'update_tag_type',
        'create_tag', 'rename_tag', 'alias_tag', 'delete_tag',
        'approve_tag', 'reject_tag', 'hide_tag', 'show_tag',
        'change_setting'
    ));
