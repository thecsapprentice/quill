-- Phase 1h: every admin-log entry says who acted and which accounts the
-- decision was against. Visible to administrators only.

-- Who acted: an administrator (named), the server itself (e.g. syncing the
-- reserved-handles file), or a writer's report (the reporter stays unnamed).
-- The administrator's login name is kept as it was, so the history still
-- reads after an account is renamed or deleted.
ALTER TABLE admin_log
    ADD COLUMN actor text,
    ADD COLUMN admin_login text;

-- Backfilling existing rows is the one change the append-only trigger must
-- allow, so it is switched off for this migration only.
ALTER TABLE admin_log DISABLE TRIGGER admin_log_append_only;
UPDATE admin_log l SET
    actor = CASE
        WHEN l.action = 'report_received' THEN 'reporter'
        WHEN l.admin_id IS NOT NULL THEN 'admin'
        ELSE 'server'
    END,
    admin_login = (SELECT a.login_name FROM accounts a WHERE a.id = l.admin_id);
ALTER TABLE admin_log ENABLE TRIGGER admin_log_append_only;

ALTER TABLE admin_log
    ALTER COLUMN actor SET NOT NULL,
    ADD CONSTRAINT admin_log_actor_check CHECK (actor IN ('admin', 'server', 'reporter')),
    ADD CONSTRAINT admin_log_admin_named CHECK (actor <> 'admin' OR admin_login IS NOT NULL);

CREATE INDEX admin_log_admin_login_idx ON admin_log (admin_login, id DESC);

-- Still append-only; the comparison covers the new columns.
CREATE OR REPLACE FUNCTION admin_log_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    -- Allow the admin reference to be cleared when an account is deleted.
    IF TG_OP = 'UPDATE'
       AND NEW.admin_id IS NULL
       AND (NEW.id, NEW.at, NEW.action, NEW.report_id, NEW.prompt_id, NEW.prompt_hash,
            NEW.reason, NEW.subject, NEW.detail, NEW.actor, NEW.admin_login)
           IS NOT DISTINCT FROM
           (OLD.id, OLD.at, OLD.action, OLD.report_id, OLD.prompt_id, OLD.prompt_hash,
            OLD.reason, OLD.subject, OLD.detail, OLD.actor, OLD.admin_login)
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'admin_log is append-only';
END;
$$;

-- Which accounts each decision was against. One action can touch many (a
-- deleted tag used by several writers); server-wide changes touch none.
CREATE TABLE admin_log_targets (
    log_id     bigint NOT NULL REFERENCES admin_log (id),
    account_id uuid   REFERENCES accounts (id) ON DELETE SET NULL,
    -- As it was, so the record survives the account's deletion.
    login      text   NOT NULL,
    PRIMARY KEY (log_id, login)
);

CREATE INDEX admin_log_targets_login_idx ON admin_log_targets (login, log_id DESC);

-- Existing entries: the accounts their notices went to, and the authors of
-- reported prompts that still exist.
INSERT INTO admin_log_targets (log_id, account_id, login)
SELECT DISTINCT n.log_id, a.id, a.login_name
FROM account_notices n JOIN accounts a ON a.id = n.account_id
ON CONFLICT DO NOTHING;
INSERT INTO admin_log_targets (log_id, account_id, login)
SELECT DISTINCT l.id, ac.id, ac.login_name
FROM admin_log l
JOIN prompts p ON p.id = l.prompt_id
JOIN actors a ON a.id = p.actor_id
JOIN accounts ac ON ac.id = a.account_id
WHERE l.action IN ('report_received', 'discard', 'delete_prompt')
ON CONFLICT DO NOTHING;

-- Append-only too, except clearing the account when it is deleted.
CREATE FUNCTION admin_log_targets_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE'
       AND NEW.account_id IS NULL
       AND (NEW.log_id, NEW.login) IS NOT DISTINCT FROM (OLD.log_id, OLD.login)
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'admin_log_targets is append-only';
END;
$$;

CREATE TRIGGER admin_log_targets_append_only
    BEFORE UPDATE OR DELETE ON admin_log_targets
    FOR EACH ROW EXECUTE FUNCTION admin_log_targets_append_only();

CREATE TRIGGER admin_log_targets_no_truncate
    BEFORE TRUNCATE ON admin_log_targets
    FOR EACH STATEMENT EXECUTE FUNCTION admin_log_targets_append_only();
