-- Phase 1d: tag administration and required tag types.

-- A required tag type (namespace): every prompt saved from now on needs at
-- least one tag of this type. Existing prompts aren't changed until edited.
ALTER TABLE tag_namespaces ADD COLUMN required boolean NOT NULL DEFAULT false;

-- The admin log now records tag administration as well as reports. Report
-- entries keep their prompt fingerprint and reason; tag entries name what
-- changed in `subject` (e.g. `genre:horror`) and `detail`.
ALTER TABLE admin_log
    ALTER COLUMN prompt_hash DROP NOT NULL,
    ALTER COLUMN reason DROP NOT NULL,
    ADD COLUMN subject text,
    ADD COLUMN detail text,
    DROP CONSTRAINT admin_log_action_check,
    ADD CONSTRAINT admin_log_action_check CHECK (action IN (
        'report_received', 'discard', 'delete_prompt',
        'create_tag_type', 'update_tag_type',
        'create_tag', 'rename_tag', 'alias_tag', 'delete_tag'
    )),
    ADD CONSTRAINT admin_log_report_fields CHECK (
        action NOT IN ('report_received', 'discard', 'delete_prompt')
        OR (prompt_hash IS NOT NULL AND reason IS NOT NULL)
    );

-- Still append-only: the comparison now covers the new columns too.
CREATE OR REPLACE FUNCTION admin_log_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    -- Allow the admin reference to be cleared when an account is deleted.
    IF TG_OP = 'UPDATE'
       AND NEW.admin_id IS NULL
       AND (NEW.id, NEW.at, NEW.action, NEW.report_id, NEW.prompt_id, NEW.prompt_hash,
            NEW.reason, NEW.subject, NEW.detail)
           IS NOT DISTINCT FROM
           (OLD.id, OLD.at, OLD.action, OLD.report_id, OLD.prompt_id, OLD.prompt_hash,
            OLD.reason, OLD.subject, OLD.detail)
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'admin_log is append-only';
END;
$$;

-- Row triggers don't fire on TRUNCATE, so refuse it separately.
CREATE TRIGGER admin_log_no_truncate
    BEFORE TRUNCATE ON admin_log
    FOR EACH STATEMENT EXECUTE FUNCTION admin_log_append_only();
