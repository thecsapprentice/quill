-- Automated moderation (#43): administrators' Rhai scripts, the common data
-- they read, the chains that run them at each hook, and what they leave in
-- the inbox, the admin log and account notices. Visible to administrators
-- only, except the notices a writer gets.

CREATE TABLE moderation_scripts (
    id          uuid        PRIMARY KEY,
    name        text        NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    source      text        NOT NULL,
    -- SHA-256 of `source`, so the log and the inbox can say which version
    -- of a script acted.
    source_hash bytea       NOT NULL,
    enabled     boolean     NOT NULL DEFAULT true,
    -- Results are logged but nothing is done: for trying a new script.
    log_only    boolean     NOT NULL DEFAULT false,
    uploaded_by uuid        REFERENCES accounts (id) ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX moderation_scripts_name_idx ON moderation_scripts (lower(name));

-- Named strings any script can read and none can change. Lists are one
-- entry per line; the server parses them (see `moderation::data`).
CREATE TABLE moderation_data (
    name        text        PRIMARY KEY CHECK (name ~ '^[a-z0-9_]{1,64}$'),
    description text        NOT NULL DEFAULT '',
    content     text        NOT NULL,
    version     integer     NOT NULL CHECK (version >= 1),
    updated_by  uuid        REFERENCES accounts (id) ON DELETE SET NULL,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

-- Every saved version, for history and undo. Undo saves an old version
-- again as a new one, so this only grows until the entry is deleted.
CREATE TABLE moderation_data_versions (
    name       text        NOT NULL REFERENCES moderation_data (name) ON DELETE CASCADE,
    version    integer     NOT NULL,
    content    text        NOT NULL,
    created_by uuid        REFERENCES accounts (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (name, version)
);

-- The entries each script reads, found in its source at upload. The
-- foreign key is what stops an entry being deleted while a script uses it.
CREATE TABLE moderation_script_data (
    script_id uuid NOT NULL REFERENCES moderation_scripts (id) ON DELETE CASCADE,
    data_name text NOT NULL REFERENCES moderation_data (name) ON DELETE RESTRICT,
    PRIMARY KEY (script_id, data_name)
);

CREATE INDEX moderation_script_data_name_idx ON moderation_script_data (data_name);

-- One chain per hook. `on_error` is what a failing script means for it.
CREATE TABLE moderation_chains (
    hook     text PRIMARY KEY CHECK (hook IN (
                 'prompt_created', 'prompt_edited', 'comment_created',
                 'tag_requested', 'prompt_reported'
             )),
    on_error text NOT NULL CHECK (on_error IN ('continue', 'reject', 'flag'))
);

-- Positions are rewritten whole when a chain is saved, so the key is
-- checked at commit rather than row by row.
CREATE TABLE moderation_chain_steps (
    hook      text    NOT NULL REFERENCES moderation_chains (hook) ON DELETE CASCADE,
    position  integer NOT NULL CHECK (position >= 0),
    script_id uuid    NOT NULL REFERENCES moderation_scripts (id) ON DELETE CASCADE,
    PRIMARY KEY (hook, position) DEFERRABLE INITIALLY DEFERRED,
    UNIQUE (hook, script_id)
);

-- A script's `flag` files a report like a writer's, with no reporter. The
-- name is kept as it was, so the inbox still reads after the script is
-- removed.
ALTER TABLE reports
    ADD COLUMN script_id   uuid REFERENCES moderation_scripts (id) ON DELETE SET NULL,
    ADD COLUMN script_name text,
    ADD CONSTRAINT reports_script_named CHECK (script_id IS NULL OR script_name IS NOT NULL);

-- At most one automated report per script for each version of a prompt or
-- comment, so an edit loop can't flood the inbox.
CREATE UNIQUE INDEX reports_one_per_script_idx
    ON reports (script_name, coalesce(prompt_id, comment_id), prompt_hash)
    WHERE script_name IS NOT NULL;

-- A script's `flag` on a requested tag marks it in the pending-tag queue.
ALTER TABLE tags
    ADD COLUMN flag_script text,
    ADD COLUMN flag_reason text;

-- `automod` acts for scripts and names the script it ran; administrators'
-- own changes to scripts, chains and data are logged as `admin`.
ALTER TABLE admin_log
    ADD COLUMN script_name text,
    DROP CONSTRAINT admin_log_actor_check,
    ADD CONSTRAINT admin_log_actor_check CHECK (actor IN ('admin', 'server', 'reporter', 'automod')),
    ADD CONSTRAINT admin_log_automod_named CHECK (actor <> 'automod' OR script_name IS NOT NULL),
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
        'automod_reject', 'automod_flag', 'automod_dismiss', 'automod_error'
    ));

-- Still append-only; the comparison covers the new column.
CREATE OR REPLACE FUNCTION admin_log_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    -- Allow the admin reference to be cleared when an account is deleted.
    IF TG_OP = 'UPDATE'
       AND NEW.admin_id IS NULL
       AND (NEW.id, NEW.at, NEW.action, NEW.report_id, NEW.prompt_id, NEW.prompt_hash,
            NEW.reason, NEW.subject, NEW.detail, NEW.actor, NEW.admin_login, NEW.comment_id,
            NEW.script_name)
           IS NOT DISTINCT FROM
           (OLD.id, OLD.at, OLD.action, OLD.report_id, OLD.prompt_id, OLD.prompt_hash,
            OLD.reason, OLD.subject, OLD.detail, OLD.actor, OLD.admin_login, OLD.comment_id,
            OLD.script_name)
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'admin_log is append-only';
END;
$$;

-- A refused submission leaves a notice holding its title and the script's
-- message, never the refused text.
ALTER TABLE account_notices
    DROP CONSTRAINT account_notices_kind_check,
    ADD CONSTRAINT account_notices_kind_check CHECK (kind IN (
        'prompt_removed',
        'tag_deleted', 'tag_rejected', 'tag_merged', 'tag_hidden',
        'tag_shown', 'tag_renamed', 'tag_approved',
        'comment_removed',
        'content_rejected'
    ));
