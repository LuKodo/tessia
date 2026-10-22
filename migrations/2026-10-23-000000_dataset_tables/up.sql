-- The prepared convenio dataset (port of Go cargue_preparacion, cargue_factura,
-- cargue_factura_linea and cargue_actual). Keyed by cargues.seq.

-- Header: one row per prepared cargue.
CREATE TABLE dataset_preparacion (
    cargue_seq         BIGINT        NOT NULL PRIMARY KEY REFERENCES cargues (seq) ON DELETE CASCADE,
    load_id            TEXT          NOT NULL,
    load_name          TEXT          NOT NULL DEFAULT '',
    load_from          DATE          NULL,
    load_to            DATE          NULL,
    load_origin        TEXT          NOT NULL DEFAULT 'unknown' CHECK (load_origin IN ('manual', 'medicar', 'unknown')),
    source             TEXT          NOT NULL DEFAULT '',
    max_amount         NUMERIC(14,2) NULL,
    automatic_subplans JSONB         NOT NULL DEFAULT '[]',
    nt_version         TEXT          NOT NULL DEFAULT '',
    prepared_format    BOOLEAN       NOT NULL DEFAULT false,
    stats              JSONB         NOT NULL DEFAULT '{}',
    skipped_lines      JSONB         NOT NULL DEFAULT '[]',
    prepared_at        TIMESTAMPTZ   NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ   NOT NULL DEFAULT now()
);

-- Every invoice of the dataset, whatever its state. `orden` is the stored
-- position (prepared, excluded and manual-review share one sequence).
-- `version` is the optimistic-locking token: 0 on first insert, +1 on every
-- update and on every re-store of the same invoice.
CREATE TABLE dataset_factura (
    cargue_seq   BIGINT      NOT NULL REFERENCES cargues (seq) ON DELETE CASCADE,
    factura      TEXT        NOT NULL,
    factura_norm TEXT        NOT NULL,
    estado       TEXT        NOT NULL CHECK (estado IN ('PREPARADA', 'EXCLUIDA', 'REVISION_MANUAL')),
    codigo       TEXT        NOT NULL DEFAULT '',
    motivo       TEXT        NOT NULL DEFAULT '',
    id_mipres    TEXT        NOT NULL DEFAULT '',
    orden        INTEGER     NOT NULL,
    conciliacion JSONB       NULL,
    version      BIGINT      NOT NULL DEFAULT 0,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (cargue_seq, factura)
);
CREATE INDEX dataset_factura_orden_idx ON dataset_factura (cargue_seq, orden);
CREATE INDEX dataset_factura_norm_idx ON dataset_factura (factura_norm, cargue_seq);

-- Lines of the prepared invoices. `data` is the whole Line as JSON (typed
-- fields plus `extra`); `total_value` is copied out for the agreement total.
CREATE TABLE dataset_linea (
    cargue_seq  BIGINT  NOT NULL,
    factura     TEXT    NOT NULL,
    orden       INTEGER NOT NULL,
    source_line INTEGER NOT NULL DEFAULT 0,
    total_value TEXT    NOT NULL DEFAULT '',
    data        JSONB   NOT NULL,
    PRIMARY KEY (cargue_seq, factura, orden),
    FOREIGN KEY (cargue_seq, factura) REFERENCES dataset_factura (cargue_seq, factura) ON DELETE CASCADE
);

-- The cargue readers resolve for a load: moved by `replace` in the same
-- transaction that commits a dataset, so an ingested-but-unprepared upload
-- never shadows the previous prepared one. A load with no pointer resolves to
-- its newest upload.
CREATE TABLE dataset_actual (
    load_id    TEXT        NOT NULL PRIMARY KEY,
    cargue_seq BIGINT      NOT NULL REFERENCES cargues (seq) ON DELETE CASCADE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
