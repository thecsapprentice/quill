-- Phase 1h: account notices. A record, readable only by the account, of
-- administrators' actions against its prompts and tags. Each points at the
-- admin-log entry it came from. Never names a reporter or an administrator.

CREATE TABLE account_notices (
    id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    account_id uuid        NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    log_id     bigint      NOT NULL REFERENCES admin_log (id),
    kind       text        NOT NULL CHECK (kind IN (
                   'prompt_removed',
                   'tag_deleted', 'tag_rejected', 'tag_merged', 'tag_hidden',
                   'tag_shown', 'tag_renamed', 'tag_approved'
               )),
    -- Kind-specific fields: e.g. {"tag": "genre:horror", "into": "...",
    -- "reason": "spam", "from": "...", "to": "..."}.
    fields     jsonb       NOT NULL DEFAULT '{}',
    -- The account's affected prompts as they were: [{"id", "title", "handle"}].
    prompts    jsonb       NOT NULL DEFAULT '[]',
    created_at timestamptz NOT NULL DEFAULT now(),
    read_at    timestamptz
);

CREATE INDEX account_notices_account_idx ON account_notices (account_id, id DESC);
CREATE INDEX account_notices_unread_idx ON account_notices (account_id) WHERE read_at IS NULL;
