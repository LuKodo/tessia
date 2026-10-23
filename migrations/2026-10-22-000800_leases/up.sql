-- Cross-host named leases (replace the .tessia-*.lock files). A lease is held
-- while expires_at is in the future; all times come from the database clock.
CREATE TABLE leases (
    name       TEXT        NOT NULL PRIMARY KEY,
    owner      TEXT        NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
