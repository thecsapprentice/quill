-- Phase 1g: reserved persona handles, managed by administrators.
--
-- word:  reserves the handle that is the word, has it as a `_`-separated
--        part, or spells it with underscores removed (stored without them).
-- exact: reserves only that handle.
-- Handles are also checked with digits read as letters. The server's own
-- name is always reserved and isn't stored here.

CREATE TABLE reserved_handles (
    pattern    text        NOT NULL CHECK (pattern ~ '^[a-z0-9_]{1,64}$'),
    kind       text        NOT NULL CHECK (kind IN ('word', 'exact')),
    -- default: Quill's starting list; admin: added in Server settings or the
    -- API; file: loaded from QUILL_RESERVED_HANDLES_FILE at startup.
    source     text        NOT NULL CHECK (source IN ('default', 'admin', 'file')),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (pattern, kind),
    CHECK (kind = 'exact' OR pattern !~ '_')
);

ALTER TABLE admin_log
    DROP CONSTRAINT admin_log_action_check,
    ADD CONSTRAINT admin_log_action_check CHECK (action IN (
        'report_received', 'discard', 'delete_prompt',
        'create_tag_type', 'update_tag_type',
        'create_tag', 'rename_tag', 'alias_tag', 'delete_tag',
        'approve_tag', 'reject_tag', 'hide_tag', 'show_tag',
        'change_setting', 'recolor_tag',
        'reserve_handles', 'unreserve_handle'
    ));
