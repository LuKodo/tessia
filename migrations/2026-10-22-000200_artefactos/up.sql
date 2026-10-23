-- Index of the objects stored in the bucket for each cargue. The bucket key is
-- never parsed back: this table is the source of truth for what exists.
CREATE TABLE artefactos (
    cargue_id    TEXT        NOT NULL,
    rel_path     TEXT        NOT NULL,
    key          TEXT        NOT NULL,
    size         BIGINT      NOT NULL CHECK (size >= 0),
    sha256       TEXT        NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    content_type TEXT        NOT NULL DEFAULT 'application/octet-stream',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (cargue_id, rel_path),
    CONSTRAINT artefactos_key_key UNIQUE (key)
);
