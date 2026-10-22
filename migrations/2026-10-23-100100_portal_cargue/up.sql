-- Mutual Ser portal follow-up of the RIPS JSON upload (`radicacion run
-- --portal-follow`): one row per invoice with the state the portal reports and
-- one row per validation finding.
CREATE TABLE portal_cargue (
    num_factura        TEXT        NOT NULL PRIMARY KEY,
    estado             TEXT        NOT NULL DEFAULT '',
    remision           TEXT        NOT NULL DEFAULT '',
    fecha_cargue       TIMESTAMPTZ NULL,
    fecha_cargue_texto TEXT        NOT NULL DEFAULT '',
    usuario            TEXT        NOT NULL DEFAULT '',
    servicios          TEXT        NULL,
    modalidad          TEXT        NOT NULL DEFAULT '',
    nombre_archivo     TEXT        NOT NULL DEFAULT '',
    id_archivo         TEXT        NOT NULL DEFAULT '',
    id_cargue          TEXT        NOT NULL DEFAULT '',
    id_evento          TEXT        NOT NULL DEFAULT '',
    tipo_evento        TEXT        NOT NULL DEFAULT '',
    tipo_error         TEXT        NOT NULL DEFAULT '',
    codigo_error       TEXT        NOT NULL DEFAULT '',
    devuelto           BOOLEAN     NOT NULL DEFAULT FALSE,
    fecha_expedicion   TEXT        NOT NULL DEFAULT '',
    receipt_name       TEXT        NOT NULL DEFAULT '',
    documento_soporte  TEXT        NULL,
    raw_json           TEXT        NULL,
    total_advertencias INTEGER     NOT NULL DEFAULT 0,
    total_errores      INTEGER     NOT NULL DEFAULT 0,
    total_hallazgos    INTEGER     NOT NULL DEFAULT 0,
    es_final           BOOLEAN     NOT NULL DEFAULT FALSE,
    consultas          INTEGER     NOT NULL DEFAULT 0,
    consultado_en      TIMESTAMPTZ NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE portal_cargue_detalle (
    id          BIGSERIAL PRIMARY KEY,
    num_factura TEXT    NOT NULL,
    orden       INTEGER NOT NULL,
    codigo      TEXT    NOT NULL DEFAULT '',
    tipo        TEXT    NOT NULL DEFAULT '',
    campo       TEXT    NOT NULL DEFAULT '',
    detalle     TEXT    NULL
);
CREATE INDEX portal_cargue_detalle_factura_idx ON portal_cargue_detalle (num_factura, orden);
