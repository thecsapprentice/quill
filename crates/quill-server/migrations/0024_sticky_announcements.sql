-- Sticky announcements (#41): each keeps its own banner at the top of the
-- board, which readers can't dismiss, until an administrator unsticks it.

ALTER TABLE announcements ADD COLUMN sticky boolean NOT NULL DEFAULT false;

CREATE INDEX announcements_sticky_idx ON announcements (created_at DESC, id DESC)
    WHERE sticky AND deleted_at IS NULL;

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
        'set_announcement_comments', 'set_announcement_sticky',
        'delete_comment'
    ));
