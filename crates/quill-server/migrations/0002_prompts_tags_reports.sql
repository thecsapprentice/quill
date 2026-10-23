-- Phase 1: prompt board, tags, maturity settings, reports and the admin log.
--
-- Prompts are public by design. Nothing here relates to writing sessions,
-- whose content the server never sees.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- Ordered: general < teen < mature < explicit.
CREATE TYPE maturity AS ENUM ('general', 'teen', 'mature', 'explicit');
CREATE TYPE prompt_state AS ENUM ('open', 'busy', 'closed');

-- Highest rating this account sees on the board. New accounts start at teen;
-- anonymous visitors always get teen.
ALTER TABLE accounts ADD COLUMN maturity_max maturity NOT NULL DEFAULT 'teen';

-- A facet of tags, e.g. genre or content-warning. Locked namespaces accept
-- only server-curated tags, whatever the server's tag policy.
CREATE TABLE tag_namespaces (
    name       text        PRIMARY KEY,
    display    text        NOT NULL,
    locked     boolean     NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE tags (
    id           bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    namespace    text        NOT NULL REFERENCES tag_namespaces (name) ON UPDATE CASCADE,
    slug         text        NOT NULL,
    display      text        NOT NULL,
    origin       text        NOT NULL CHECK (origin IN ('server', 'user')),
    -- Host that defined the tag; '' for this server. Remote tags arrive with
    -- federation in Phase 3.
    instance     text        NOT NULL DEFAULT '',
    -- Set when an admin merges this tag into another (an alias).
    canonical_id bigint      REFERENCES tags (id),
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (instance, namespace, slug),
    CHECK (canonical_id IS NULL OR canonical_id <> id)
);

CREATE INDEX tags_slug_trgm_idx ON tags USING gin (slug gin_trgm_ops);
CREATE INDEX tags_display_trgm_idx ON tags USING gin (lower(display) gin_trgm_ops);

CREATE TABLE prompts (
    id          uuid         PRIMARY KEY,
    actor_id    uuid         NOT NULL REFERENCES actors (id) ON DELETE CASCADE,
    title       text         NOT NULL,
    body_md     text         NOT NULL,
    body_html   text         NOT NULL,
    maturity    maturity     NOT NULL,
    state       prompt_state NOT NULL DEFAULT 'open',
    -- Canonical tag ids; aliases are resolved when a prompt is written.
    tag_ids     bigint[]     NOT NULL DEFAULT '{}',
    search      tsvector     GENERATED ALWAYS AS (
                    setweight(to_tsvector('simple', title), 'A')
                    || setweight(to_tsvector('simple', body_md), 'B')
                ) STORED,
    created_at  timestamptz  NOT NULL DEFAULT now(),
    updated_at  timestamptz  NOT NULL DEFAULT now(),
    -- Set when an administrator removes the prompt after a report. The author
    -- still sees it, marked as removed; nobody else does.
    removed_at  timestamptz
);

CREATE INDEX prompts_tag_ids_idx ON prompts USING gin (tag_ids);
CREATE INDEX prompts_search_idx ON prompts USING gin (search);
CREATE INDEX prompts_board_idx ON prompts (created_at DESC, id DESC) WHERE removed_at IS NULL;
CREATE INDEX prompts_actor_idx ON prompts (actor_id);

CREATE TYPE report_reason AS ENUM ('illegal', 'spam', 'harassment', 'wrong-maturity', 'other');

CREATE TABLE reports (
    id               uuid          PRIMARY KEY,
    prompt_id        uuid          REFERENCES prompts (id) ON DELETE SET NULL,
    -- For abuse control only; never shown to the prompt's author.
    reporter_account uuid          REFERENCES accounts (id) ON DELETE SET NULL,
    reason           report_reason NOT NULL,
    details          text,
    -- SHA-256 of the prompt as reported, so the log can show what was
    -- reported without keeping its text.
    prompt_hash      bytea         NOT NULL,
    created_at       timestamptz   NOT NULL DEFAULT now(),
    resolved_at      timestamptz,
    resolution       text          CHECK (resolution IN ('discard', 'delete')),
    resolved_by      uuid          REFERENCES accounts (id) ON DELETE SET NULL
);

CREATE INDEX reports_open_idx ON reports (created_at) WHERE resolved_at IS NULL;

-- Append-only record of reports received and actions taken.
CREATE TABLE admin_log (
    id          bigint        GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at          timestamptz   NOT NULL DEFAULT now(),
    action      text          NOT NULL CHECK (action IN ('report_received', 'discard', 'delete_prompt')),
    report_id   uuid,
    prompt_id   uuid,
    prompt_hash bytea         NOT NULL,
    reason      report_reason NOT NULL,
    admin_id    uuid          REFERENCES accounts (id) ON DELETE SET NULL
);

CREATE FUNCTION admin_log_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    -- Allow the admin reference to be cleared when an account is deleted.
    IF TG_OP = 'UPDATE'
       AND NEW.admin_id IS NULL
       AND (NEW.id, NEW.at, NEW.action, NEW.report_id, NEW.prompt_id, NEW.prompt_hash, NEW.reason)
           IS NOT DISTINCT FROM
           (OLD.id, OLD.at, OLD.action, OLD.report_id, OLD.prompt_id, OLD.prompt_hash, OLD.reason)
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'admin_log is append-only';
END;
$$;

CREATE TRIGGER admin_log_append_only
    BEFORE UPDATE OR DELETE ON admin_log
    FOR EACH ROW EXECUTE FUNCTION admin_log_append_only();

-- Default facets. Administrators can add namespaces and tags.
INSERT INTO tag_namespaces (name, display, locked) VALUES
    ('genre',           'Genre',           false),
    ('content-warning', 'Content warning', true),
    ('format',          'Format',          true),
    ('length',          'Length',          true),
    ('theme',           'Theme',           false);

INSERT INTO tags (namespace, slug, display, origin) VALUES
    ('genre', 'fantasy', 'Fantasy', 'server'),
    ('genre', 'science-fiction', 'Science fiction', 'server'),
    ('genre', 'horror', 'Horror', 'server'),
    ('genre', 'mystery', 'Mystery', 'server'),
    ('genre', 'romance', 'Romance', 'server'),
    ('genre', 'thriller', 'Thriller', 'server'),
    ('genre', 'historical', 'Historical', 'server'),
    ('genre', 'literary', 'Literary', 'server'),
    ('genre', 'comedy', 'Comedy', 'server'),
    ('genre', 'slice-of-life', 'Slice of life', 'server'),
    ('content-warning', 'violence', 'Violence', 'server'),
    ('content-warning', 'gore', 'Gore', 'server'),
    ('content-warning', 'death', 'Death', 'server'),
    ('content-warning', 'abuse', 'Abuse', 'server'),
    ('content-warning', 'self-harm', 'Self-harm', 'server'),
    ('content-warning', 'substance-use', 'Substance use', 'server'),
    ('content-warning', 'sexual-content', 'Sexual content', 'server'),
    ('format', 'prose', 'Prose', 'server'),
    ('format', 'poetry', 'Poetry', 'server'),
    ('format', 'script', 'Script', 'server'),
    ('format', 'epistolary', 'Epistolary', 'server'),
    ('format', 'role-play', 'Role-play', 'server'),
    ('length', 'flash', 'Flash', 'server'),
    ('length', 'short', 'Short', 'server'),
    ('length', 'long', 'Long', 'server'),
    ('length', 'open-ended', 'Open-ended', 'server');
