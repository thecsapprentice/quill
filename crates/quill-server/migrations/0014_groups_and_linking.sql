-- Phase 2d: MLS groups' message logs (the epoch sequencer) and the
-- short-lived mailbox for linking a new device.
--
-- Everything stored here is MLS ciphertext or public key material; the
-- server reads only each message's clear header (group id, epoch, content
-- type).

CREATE TABLE mls_groups (
    group_id   bytea  PRIMARY KEY CHECK (octet_length(group_id) BETWEEN 16 AND 64),
    -- 'personal': a login's own devices. Sessions ('session') come in 2f.
    kind       text   NOT NULL CHECK (kind IN ('personal')),
    -- The login a personal group belongs to.
    account_id uuid   REFERENCES accounts (id) ON DELETE CASCADE,
    -- The epoch the next commit must be for.
    epoch      bigint NOT NULL DEFAULT 0 CHECK (epoch >= 0),
    CHECK (kind <> 'personal' OR account_id IS NOT NULL)
);

CREATE UNIQUE INDEX mls_groups_one_personal_idx ON mls_groups (account_id)
    WHERE kind = 'personal';

-- Each group's messages in the order the server accepted them.
CREATE TABLE group_messages (
    group_id  bytea   NOT NULL REFERENCES mls_groups (group_id) ON DELETE CASCADE,
    seq       bigint  NOT NULL CHECK (seq > 0),
    epoch     bigint  NOT NULL,
    is_commit boolean NOT NULL,
    data      bytea   NOT NULL,
    PRIMARY KEY (group_id, seq)
);

-- Device linking (protocol draft, section 6): a new device leaves its
-- KeyPackage here; a device already in use answers with a Welcome.
CREATE TABLE link_mailbox (
    link_id     bytea       PRIMARY KEY CHECK (octet_length(link_id) = 16),
    account_id  uuid        NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    key_package bytea       NOT NULL,
    welcome     bytea,
    -- Where the new device starts reading the personal group's log.
    from_seq    bigint,
    expires_at  timestamptz NOT NULL
);
