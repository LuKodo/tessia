-- Mutual Ser filing pipeline state, one row per normalized invoice.
CREATE TABLE radicacion_estado (
    invoice          TEXT        NOT NULL PRIMARY KEY,
    folder           TEXT        NOT NULL DEFAULT '',
    step             SMALLINT    NOT NULL CHECK (step BETWEEN 1 AND 4),
    status           TEXT        NOT NULL,
    upload_id        TEXT        NOT NULL DEFAULT '',
    sent_at          TIMESTAMPTZ NULL,
    sends            INTEGER     NOT NULL DEFAULT 0,
    polls            INTEGER     NOT NULL DEFAULT 0,
    step1_reloaded   BOOLEAN     NOT NULL DEFAULT FALSE,
    reload_for_step2 BOOLEAN     NOT NULL DEFAULT FALSE,
    next_attempt_at  TIMESTAMPTZ NULL,
    last_detail      TEXT        NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX radicacion_estado_status_idx ON radicacion_estado (status, step);
