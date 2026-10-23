-- Phase 2j: encrypted backups. Each is an age file the server can't open,
-- uploaded in parts under a new generation and committed as a whole, so a
-- backup is replaced only once the new one is complete.

CREATE TABLE backup_parts (
    account_id  uuid        NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    generation  uuid        NOT NULL,
    part        integer     NOT NULL CHECK (part >= 0),
    data        bytea       NOT NULL,
    uploaded_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, generation, part)
);

-- The committed backup: one per login.
CREATE TABLE backups (
    account_id uuid        PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    generation uuid        NOT NULL,
    parts      integer     NOT NULL CHECK (parts > 0),
    bytes      bigint      NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
