-- Catalog of the distinct portal findings (Go `portal_hallazgo`): filled by
-- `PortalStore::save` with `INSERT ... ON CONFLICT DO NOTHING` (the first
-- detail ever seen wins); only `genera_glosa`, `devolucion` and `descripcion`
-- are edited by the user through `PUT /api/hallazgos/{codigo}`.
CREATE TABLE portal_hallazgo (
    codigo       TEXT        NOT NULL PRIMARY KEY,
    detalle      TEXT        NULL,
    genera_glosa BOOLEAN     NOT NULL DEFAULT FALSE,
    devolucion   BOOLEAN     NOT NULL DEFAULT FALSE,
    descripcion  TEXT        NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Seed from the findings already stored (codes are trimmed and cut to 100
-- characters, like `PortalStore::save` registers them).
INSERT INTO portal_hallazgo (codigo, detalle)
SELECT left(btrim(codigo), 100), MIN(detalle)
FROM portal_cargue_detalle
WHERE btrim(codigo) <> ''
GROUP BY left(btrim(codigo), 100)
ON CONFLICT (codigo) DO NOTHING;
