-- Phase 0: accounts, personas (actors) and API tokens.
--
-- Privacy notes:
--  * login_name is private to the account and never federated or shown.
--  * No IP addresses, user agents or email addresses are stored.
--  * API tokens are stored only as SHA-256 hashes.

CREATE TABLE accounts (
    id            uuid        PRIMARY KEY,
    login_name    text        NOT NULL UNIQUE,
    password_hash text        NOT NULL,
    is_admin      boolean     NOT NULL DEFAULT false,
    created_at    timestamptz NOT NULL DEFAULT now()
);

-- A persona is a public ActivityPub actor. One account may own several;
-- they are never linked in any public or federated view.
CREATE TABLE actors (
    id                 uuid        PRIMARY KEY,
    account_id         uuid        NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    preferred_username text        NOT NULL UNIQUE,
    display_name       text,
    summary            text,
    created_at         timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX actors_account_id_idx ON actors (account_id);

CREATE TABLE api_tokens (
    token_hash bytea       PRIMARY KEY,
    account_id uuid        NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);

CREATE INDEX api_tokens_account_id_idx ON api_tokens (account_id);
CREATE INDEX api_tokens_expires_at_idx ON api_tokens (expires_at);
