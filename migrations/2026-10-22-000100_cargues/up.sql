-- One row per uploaded convenio CSV. `id` is the load key (the CSV stem,
-- see domain::cargue); the same load may receive different bytes over time, so
-- the primary key is a sequence and (id, sha256) is unique. The current
-- upload of a load is the one with the highest `seq`.
CREATE TABLE cargues (
    seq         BIGSERIAL PRIMARY KEY,
    id          TEXT        NOT NULL,
    sha256      TEXT        NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    name        TEXT        NOT NULL,
    origin      TEXT        NOT NULL DEFAULT 'unknown' CHECK (origin IN ('manual', 'medicar', 'unknown')),
    date_from   DATE        NULL,
    date_to     DATE        NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT cargues_id_sha256_key UNIQUE (id, sha256)
);
CREATE INDEX cargues_created_at_idx ON cargues (created_at DESC);
