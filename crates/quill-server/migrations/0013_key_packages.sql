-- Phase 2c: each device's pool of MLS KeyPackages (public, signed by the
-- device). One-time packages are removed as they're claimed; the
-- last-resort one is reused when the pool is empty.

CREATE TABLE key_packages (
    -- Internal only, never published.
    id          bigint  GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    device_id   uuid    NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    -- The MLSMessage as uploaded.
    data        bytea   NOT NULL,
    last_resort boolean NOT NULL,
    -- From the package's own lifetime (seconds since the Unix epoch), so
    -- expired packages can be dropped.
    not_after   bigint  NOT NULL
);

CREATE INDEX key_packages_pool_idx ON key_packages (device_id, id) WHERE NOT last_resort;
CREATE UNIQUE INDEX key_packages_last_resort_idx ON key_packages (device_id) WHERE last_resort;
