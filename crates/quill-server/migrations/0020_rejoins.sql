-- Phase 2j: rejoin requests. A restored device asks the other writer's
-- client to add it to a session; the request is signed by the persona's
-- identity key (checked here and by every client). One per persona per
-- session: a newer one replaces it.

CREATE TABLE session_rejoins (
    id         uuid        PRIMARY KEY,
    group_id   bytea       NOT NULL REFERENCES sessions (group_id) ON DELETE CASCADE,
    actor_id   uuid        NOT NULL REFERENCES actors (id) ON DELETE CASCADE,
    request    bytea       NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (group_id, actor_id)
);
