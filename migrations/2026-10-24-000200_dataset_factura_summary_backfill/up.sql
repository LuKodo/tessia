-- Backfills the summary columns of the invoices stored before they existed,
-- from dataset_linea. The rules are those of stores::dataset::summary:
-- the first line whose trimmed value is not blank wins, the date is cut to 10
-- characters, and the total follows parse_agreement_amount ("1.234,56" with a
-- comma uses dot thousands, otherwise "1234.56"; at most 12 integer digits and
-- 2 decimals) and is NULL when any line total is unreadable.
-- It only rewrites prepared invoices that still have empty columns, so running
-- it again (the test suite does) is harmless.

UPDATE dataset_factura f
SET fecha_factura_txt = COALESCE((
        SELECT left(t.v, 10)
        FROM (SELECT regexp_replace(COALESCE(l.data ->> 'date', ''), '^\s+|\s+$', '', 'g') AS v, l.orden
              FROM dataset_linea l
              WHERE l.cargue_seq = f.cargue_seq AND l.factura = f.factura) t
        WHERE t.v <> ''
        ORDER BY t.orden
        LIMIT 1), ''),
    ssc = COALESCE((
        SELECT t.v
        FROM (SELECT regexp_replace(COALESCE(l.data ->> 'ssc', ''), '^\s+|\s+$', '', 'g') AS v, l.orden
              FROM dataset_linea l
              WHERE l.cargue_seq = f.cargue_seq AND l.factura = f.factura) t
        WHERE t.v <> ''
        ORDER BY t.orden
        LIMIT 1), ''),
    subplan = COALESCE((
        SELECT t.v
        FROM (SELECT regexp_replace(COALESCE(l.data ->> 'subplan', ''), '^\s+|\s+$', '', 'g') AS v, l.orden
              FROM dataset_linea l
              WHERE l.cargue_seq = f.cargue_seq AND l.factura = f.factura) t
        WHERE t.v <> ''
        ORDER BY t.orden
        LIMIT 1), ''),
    autorizacion = COALESCE((
        SELECT t.v
        FROM (SELECT regexp_replace(COALESCE(l.data ->> 'authorization', ''), '^\s+|\s+$', '', 'g') AS v, l.orden
              FROM dataset_linea l
              WHERE l.cargue_seq = f.cargue_seq AND l.factura = f.factura) t
        WHERE t.v <> ''
        ORDER BY t.orden
        LIMIT 1), '')
WHERE f.estado = 'PREPARADA'
  AND f.fecha_factura_txt = '' AND f.ssc = '' AND f.subplan = '' AND f.autorizacion = '';

UPDATE dataset_factura f
SET valor_convenio = s.total
FROM (
    SELECT t.cargue_seq, t.factura,
           CASE WHEN bool_and(COALESCE(t.ok, false))
                     AND abs(sum(CASE WHEN t.ok THEN t.v::numeric END)) < 10000000000000000
                THEN sum(CASE WHEN t.ok THEN t.v::numeric END)
           END AS total
    FROM (
        SELECT l.cargue_seq, l.factura, p.v,
               (p.v ~ '^[0-9]{1,12}(\.[0-9]{1,2})?$') AS ok
        FROM dataset_linea l
        CROSS JOIN LATERAL (
            SELECT CASE
                       WHEN q.trimmed LIKE '%,%' THEN
                           CASE WHEN q.trimmed ~ '^[0-9]+(\.[0-9]{3})*(,[0-9]{1,2})?$'
                                THEN replace(replace(q.trimmed, '.', ''), ',', '.')
                           END
                       ELSE q.trimmed
                   END AS v
            FROM (SELECT regexp_replace(l.total_value, '^\s+|\s+$', '', 'g') AS trimmed) q
        ) p
    ) t
    GROUP BY t.cargue_seq, t.factura
) s
WHERE f.cargue_seq = s.cargue_seq AND f.factura = s.factura
  AND f.estado = 'PREPARADA' AND f.valor_convenio IS NULL;
