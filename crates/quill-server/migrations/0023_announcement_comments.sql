-- Comments on announcements (#41): flat, oldest first, written as a
-- persona. Moderated through the same reports, inbox and admin log as
-- prompts.

CREATE TABLE announcement_comments (
    id              uuid          PRIMARY KEY,
    -- Announcements are only ever soft-deleted, so this never dangles.
    announcement_id uuid          NOT NULL REFERENCES announcements (id),
    actor_id        uuid          NOT NULL REFERENCES actors (id) ON DELETE CASCADE,
    body_md         text          NOT NULL,
    body_html       text          NOT NULL,
    created_at      timestamptz   NOT NULL DEFAULT now(),
    -- Set when an administrator removes the comment. Its author still sees
    -- it, marked as removed; nobody else does.
    removed_at      timestamptz,
    removed_reason  report_reason,
    CHECK ((removed_at IS NULL) = (removed_reason IS NULL))
);

CREATE INDEX announcement_comments_thread_idx
    ON announcement_comments (announcement_id, created_at, id);
CREATE INDEX announcement_comments_actor_idx ON announcement_comments (actor_id);

-- A report is about a prompt or a comment. Either reference is cleared when
-- its author deletes the thing, so `kind` keeps what the report was about.
ALTER TABLE reports
    ADD COLUMN comment_id uuid REFERENCES announcement_comments (id) ON DELETE SET NULL,
    ADD COLUMN kind text NOT NULL DEFAULT 'prompt' CHECK (kind IN ('prompt', 'comment')),
    ADD CONSTRAINT reports_one_subject CHECK (
        (kind = 'prompt' AND comment_id IS NULL)
        OR (kind = 'comment' AND prompt_id IS NULL)
    );

-- The admin log names the comment an entry concerns. No foreign key: the
-- log outlives the comment.
ALTER TABLE admin_log
    ADD COLUMN comment_id uuid,
    DROP CONSTRAINT admin_log_action_check,
    ADD CONSTRAINT admin_log_action_check CHECK (action IN (
        'report_received', 'discard', 'delete_prompt',
        'create_tag_type', 'update_tag_type',
        'create_tag', 'rename_tag', 'alias_tag', 'delete_tag',
        'approve_tag', 'reject_tag', 'hide_tag', 'show_tag',
        'change_setting', 'recolor_tag',
        'reserve_handles', 'unreserve_handle',
        'post_announcement', 'edit_announcement', 'delete_announcement',
        'set_announcement_comments',
        'delete_comment'
    )),
    DROP CONSTRAINT admin_log_report_fields,
    ADD CONSTRAINT admin_log_report_fields CHECK (
        action NOT IN ('report_received', 'discard', 'delete_prompt', 'delete_comment')
        OR (prompt_hash IS NOT NULL AND reason IS NOT NULL)
    );

-- Still append-only; the comparison covers the new column.
CREATE OR REPLACE FUNCTION admin_log_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    -- Allow the admin reference to be cleared when an account is deleted.
    IF TG_OP = 'UPDATE'
       AND NEW.admin_id IS NULL
       AND (NEW.id, NEW.at, NEW.action, NEW.report_id, NEW.prompt_id, NEW.prompt_hash,
            NEW.reason, NEW.subject, NEW.detail, NEW.actor, NEW.admin_login, NEW.comment_id)
           IS NOT DISTINCT FROM
           (OLD.id, OLD.at, OLD.action, OLD.report_id, OLD.prompt_id, OLD.prompt_hash,
            OLD.reason, OLD.subject, OLD.detail, OLD.actor, OLD.admin_login, OLD.comment_id)
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'admin_log is append-only';
END;
$$;

ALTER TABLE account_notices
    DROP CONSTRAINT account_notices_kind_check,
    ADD CONSTRAINT account_notices_kind_check CHECK (kind IN (
        'prompt_removed',
        'tag_deleted', 'tag_rejected', 'tag_merged', 'tag_hidden',
        'tag_shown', 'tag_renamed', 'tag_approved',
        'comment_removed'
    ));
