-- Server rules (#42): a Markdown page the administrators keep, which
-- accounts may have to accept on joining, after a change, or both.

-- Longer documents the administrators publish, by name. Only `rules` for
-- now; an "about" page could follow without another migration.
CREATE TABLE server_pages (
    name           text        PRIMARY KEY,
    body_md        text        NOT NULL,
    -- Rendered with the same sanitizer as prompts.
    body_html      text        NOT NULL,
    -- Goes up by one on every save.
    version        integer     NOT NULL,
    -- The latest version accounts must accept, and when it was saved.
    -- A "minor edit" leaves both as they were.
    accept_version integer     NOT NULL,
    accept_since   timestamptz NOT NULL,
    updated_at     timestamptz NOT NULL DEFAULT now(),
    updated_by     uuid        REFERENCES accounts (id) ON DELETE SET NULL
);

-- The newest version of the rules each account accepted.
CREATE TABLE rules_acceptances (
    account_id  uuid        PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    version     integer     NOT NULL,
    accepted_at timestamptz NOT NULL DEFAULT now()
);

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
        'delete_comment',
        'upload_script', 'replace_script', 'remove_script', 'set_script_mode',
        'update_chain',
        'save_moderation_data', 'delete_moderation_data',
        'automod_reject', 'automod_flag', 'automod_dismiss', 'automod_error',
        'update_rules'
    ));
