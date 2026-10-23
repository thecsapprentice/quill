-- Phase 2i: encrypted attachments.
--
-- A blob is ciphertext padded to a size bucket. The key travels inside the
-- message that carries it; the server keeps only a hash of the capability
-- token needed to fetch it.

CREATE TABLE blobs (
    -- Random (v4), never time-ordered.
    id          uuid        PRIMARY KEY,
    group_id    bytea       NOT NULL REFERENCES sessions (group_id) ON DELETE CASCADE,
    -- The uploading login, for its storage quota.
    account_id  uuid        NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    token_hash  bytea       NOT NULL CHECK (octet_length(token_hash) = 32),
    size        bigint      NOT NULL CHECK (size > 0),
    data        bytea       NOT NULL,
    -- The log position of the message carrying it, once posted. A blob is
    -- deleted with that message; one no message names goes after a day.
    seq         bigint,
    uploaded_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX blobs_group_idx ON blobs (group_id, seq);
CREATE INDEX blobs_account_idx ON blobs (account_id);
