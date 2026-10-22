-- State of each (invoice, phase). `version` is the optimistic-concurrency
-- token: every transition bumps it and claims/outcomes are compare-and-set on
-- it. `claimed_by`/`claimed_at` identify the worker of an EN_PROGRESO row so a
-- crashed worker's claim can be taken over after a TTL.
CREATE TABLE proceso_estado (
    num_factura     TEXT        NOT NULL,
    fase            TEXT        NOT NULL,
    cargue_id       TEXT        NULL,
    estado          TEXT        NOT NULL,
    motivo          TEXT        NOT NULL DEFAULT '',
    intentos        INTEGER     NOT NULL DEFAULT 0,
    version         BIGINT      NOT NULL DEFAULT 0,
    claimed_by      TEXT        NULL,
    claimed_at      TIMESTAMPTZ NULL,
    next_attempt_at TIMESTAMPTZ NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (num_factura, fase)
);
CREATE INDEX proceso_estado_fase_estado_idx ON proceso_estado (fase, estado);
CREATE INDEX proceso_estado_cargue_idx ON proceso_estado (cargue_id, fase);
