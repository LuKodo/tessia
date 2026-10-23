-- XML total read from an invoice ZIP, kept so the value check still has it
-- when the ZIP is gone. `num_factura` is the normalized invoice. The xml_*
-- columns belong to the tracking view (a different rule than the gate's).
CREATE TABLE factura_valor_xml (
    num_factura     TEXT          NOT NULL PRIMARY KEY,
    valor_total_xml NUMERIC(18,2) NULL,
    fuente          TEXT          NULL,
    sincronizado_at TIMESTAMPTZ   NULL,
    ruta_zip        TEXT          NULL,
    xml_valor       NUMERIC(18,2) NULL,
    xml_mtime       BIGINT        NULL,
    xml_size        BIGINT        NULL
);
