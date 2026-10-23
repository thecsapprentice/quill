-- Browser sign-in sessions for the server-rendered board.
--
-- The session token lives only in an HttpOnly cookie; the database keeps its
-- SHA-256 hash. Each session has its own CSRF token, which every form posted
-- by a signed-in visitor must echo back.

CREATE TABLE web_sessions (
    token_hash bytea       PRIMARY KEY,
    account_id uuid        NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    csrf_token text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);

CREATE INDEX web_sessions_account_id_idx ON web_sessions (account_id);
CREATE INDEX web_sessions_expires_at_idx ON web_sessions (expires_at);
