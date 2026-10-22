-- Stored summary columns of a prepared invoice (port of the Go columns
-- cargue_factura.fecha_factura_txt, ssc, subplan, autorizacion and
-- valor_convenio). The dashboard filters, sorts and sums on them without
-- reading dataset_linea.data. They are filled by DatasetStore::replace and
-- update_invoices (the single Rust implementation is
-- stores::dataset::summary::summarize) and backfilled by the next migration.
--
--   fecha_factura_txt  first non-blank line `date`, trimmed, cut to 10 characters
--   ssc, subplan, autorizacion  first non-blank line value, trimmed
--   valor_convenio     sum of the line totals (parse_agreement_amount rules);
--                      NULL when the invoice has no lines or a total is unreadable
--
-- Excluded and manual-review invoices have no lines: their columns stay empty.
ALTER TABLE dataset_factura
    ADD COLUMN fecha_factura_txt TEXT          NOT NULL DEFAULT '',
    ADD COLUMN ssc               TEXT          NOT NULL DEFAULT '',
    ADD COLUMN subplan           TEXT          NOT NULL DEFAULT '',
    ADD COLUMN autorizacion      TEXT          NOT NULL DEFAULT '',
    ADD COLUMN valor_convenio    NUMERIC(18,2) NULL;
