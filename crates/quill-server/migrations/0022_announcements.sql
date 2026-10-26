-- Announcements (#41): news from the server's administrators, shown on the
-- board. Posted as the server, not as a persona; the admin log records
-- which administrator acted.

CREATE TABLE announcements (
    id            uuid        PRIMARY KEY,
    title         text        NOT NULL,
    body_md       text        NOT NULL,
    -- Rendered with the same sanitizer as prompts.
    body_html     text        NOT NULL,
    -- Whether writers may comment. Comments themselves arrive separately.
    comments_open boolean     NOT NULL DEFAULT true,
    created_by    uuid        REFERENCES accounts (id) ON DELETE SET NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    -- A soft delete, so comments and log entries keep their reference.
    -- Deleted announcements show nowhere.
    deleted_at    timestamptz
);

CREATE INDEX announcements_list_idx ON announcements (created_at DESC, id DESC)
    WHERE deleted_at IS NULL;

ALTER TABLE admin_log
    DROP CONSTRAINT admin_log_action_check,
    ADD CONSTRAINT admin_log_action_check CHECK (action IN (
        'report_received', 'discard', 'delete_prompt',
        'create_tag_type', 'update_tag_type',
        'create_tag', 'rename_tag', 'alias_tag', 'delete_tag',
        'approve_tag', 'reject_tag', 'hide_tag', 'show_tag',
        'change_setting', 'recolor_tag',
        'reserve_handles', 'unreserve_handle',
        'post_announcement', 'edit_announcement', 'delete_announcement',
        'set_announcement_comments'
    ));
