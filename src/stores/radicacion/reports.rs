//! Read models of the dashboard reports (manual-review report and filed
//! invoices) that join the dataset, `proceso_estado` and `radicacion_estado`.
//! Read-only: nothing here writes.

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Text, Timestamptz};

use crate::db::{DbPool, DbResult};
use crate::stores::chunks;

/// The cargue the dashboard reads for a load id: the one `dataset_actual`
/// points to, else the newest upload.
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct CargueRef {
    #[diesel(sql_type = BigInt)]
    pub seq: i64,
    #[diesel(sql_type = Text)]
    pub id: String,
    /// `dataset_preparacion.load_name`, else `cargues.name`.
    #[diesel(sql_type = Text)]
    pub name: String,
}

/// One invoice row of a stored dataset, whatever its state.
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct DatasetEntry {
    #[diesel(sql_type = Text)]
    pub invoice: String,
    /// `PREPARADA`, `EXCLUIDA` or `REVISION_MANUAL`.
    #[diesel(sql_type = Text)]
    pub state: String,
    #[diesel(sql_type = Text)]
    pub code: String,
    #[diesel(sql_type = Text)]
    pub reason: String,
}

/// A prepared invoice whose four filing steps are confirmed.
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct FiledInvoice {
    #[diesel(sql_type = BigInt)]
    pub cargue_seq: i64,
    #[diesel(sql_type = Text)]
    pub invoice: String,
    /// `radicacion_estado.updated_at`: when step 4 was confirmed.
    #[diesel(sql_type = Timestamptz)]
    pub filed_at: DateTime<Utc>,
    #[diesel(sql_type = Text)]
    pub last_detail: String,
}

/// The dataset-line fields an invoice summary is derived from.
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct LineFacts {
    #[diesel(sql_type = Text)]
    pub invoice: String,
    #[diesel(sql_type = Text)]
    pub date: String,
    #[diesel(sql_type = Text)]
    pub ssc: String,
    #[diesel(sql_type = Text)]
    pub authorization: String,
    #[diesel(sql_type = Text)]
    pub total_value: String,
}

/// The reason a phase recorded for an invoice, with its time.
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct PhaseNote {
    /// Normalized invoice (`proceso_estado.num_factura`).
    #[diesel(sql_type = Text)]
    pub invoice: String,
    #[diesel(sql_type = Text)]
    pub reason: String,
    #[diesel(sql_type = Timestamptz)]
    pub at: DateTime<Utc>,
}

const CARGUE_COLUMNS: &str = "c.seq AS seq, c.id AS id, \
     COALESCE(NULLIF(p.load_name, ''), c.name) AS name";

/// The current cargue of a load (`dataset_actual`, else the newest upload).
const CURRENT_OF_LOAD: &str = "c.seq = COALESCE( \
     (SELECT a.cargue_seq FROM dataset_actual a WHERE a.load_id = c.id), \
     (SELECT MAX(n.seq) FROM cargues n WHERE n.id = c.id))";

/// Read store behind `GET /api/cargues/{id}/informe` and
/// `GET /api/reportes/radicadas`.
#[derive(Clone)]
pub struct ReportStore {
    pool: DbPool,
}

impl ReportStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// The current cargue of the load `id`; `None` when no upload has that id.
    pub fn current_cargue(&self, id: &str) -> DbResult<Option<CargueRef>> {
        self.pool.run("report current cargue", |conn| {
            Ok(diesel::sql_query(format!(
                "SELECT {CARGUE_COLUMNS} FROM cargues c \
                 LEFT JOIN dataset_preparacion p ON p.cargue_seq = c.seq \
                 WHERE c.id = $1 AND {CURRENT_OF_LOAD}"
            ))
            .bind::<Text, _>(id)
            .get_result::<CargueRef>(conn)
            .optional()?)
        })
    }

    /// Every load's current cargue, ordered by id.
    pub fn current_cargues(&self) -> DbResult<Vec<CargueRef>> {
        self.pool.run("report current cargues", |conn| {
            Ok(diesel::sql_query(format!(
                "SELECT {CARGUE_COLUMNS} FROM cargues c \
                 LEFT JOIN dataset_preparacion p ON p.cargue_seq = c.seq \
                 WHERE {CURRENT_OF_LOAD} ORDER BY c.id"
            ))
            .load::<CargueRef>(conn)?)
        })
    }

    /// Every invoice of the cargue's dataset in stored order (no lines).
    pub fn dataset_entries(&self, cargue_seq: i64) -> DbResult<Vec<DatasetEntry>> {
        self.pool.run("report dataset entries", |conn| {
            Ok(diesel::sql_query(
                "SELECT factura AS invoice, estado AS state, codigo AS code, motivo AS reason \
                 FROM dataset_factura WHERE cargue_seq = $1 ORDER BY orden, factura",
            )
            .bind::<BigInt, _>(cargue_seq)
            .load::<DatasetEntry>(conn)?)
        })
    }

    /// The prepared invoices of the given cargues whose filing is complete
    /// (`radicacion_estado` at step 4, confirmed). Unordered.
    pub fn filed_invoices(&self, cargue_seqs: &[i64]) -> DbResult<Vec<FiledInvoice>> {
        let seqs = cargue_seqs.to_vec();
        self.pool.run("report filed invoices", |conn| {
            Ok(diesel::sql_query(
                "SELECT f.cargue_seq AS cargue_seq, f.factura AS invoice, \
                        r.updated_at AS filed_at, COALESCE(r.last_detail, '') AS last_detail \
                 FROM dataset_factura f \
                 JOIN radicacion_estado r ON r.invoice = f.factura_norm \
                 WHERE f.cargue_seq = ANY($1) AND f.estado = 'PREPARADA' \
                   AND r.step = 4 AND r.status = 'CONFIRMED'",
            )
            .bind::<Array<BigInt>, _>(&seqs)
            .load::<FiledInvoice>(conn)?)
        })
    }

    /// The summary fields of the lines of `invoices` (as stored in the
    /// dataset) of one cargue, ordered by invoice then line.
    pub fn line_facts(&self, cargue_seq: i64, invoices: &[String]) -> DbResult<Vec<LineFacts>> {
        let mut out = Vec::new();
        for part in chunks(invoices) {
            let names = part.to_vec();
            out.extend(self.pool.run("report line facts", |conn| {
                Ok(diesel::sql_query(
                    "SELECT factura AS invoice, COALESCE(data->>'date', '') AS date, \
                            COALESCE(data->>'ssc', '') AS ssc, \
                            COALESCE(data->>'authorization', '') AS authorization, \
                            total_value \
                     FROM dataset_linea WHERE cargue_seq = $1 AND factura = ANY($2) \
                     ORDER BY factura, orden",
                )
                .bind::<BigInt, _>(cargue_seq)
                .bind::<Array<Text>, _>(&names)
                .load::<LineFacts>(conn)?)
            })?);
        }
        Ok(out)
    }

    /// Every phase reason recorded for the (normalized) `invoices`, whatever
    /// the phase or state.
    pub fn phase_notes(&self, invoices: &[String]) -> DbResult<Vec<PhaseNote>> {
        let mut out = Vec::new();
        for part in chunks(invoices) {
            let names = part.to_vec();
            out.extend(self.pool.run("report phase notes", |conn| {
                Ok(diesel::sql_query(
                    "SELECT num_factura AS invoice, motivo AS reason, updated_at AS at \
                     FROM proceso_estado WHERE num_factura = ANY($1) AND motivo <> ''",
                )
                .bind::<Array<Text>, _>(&names)
                .load::<PhaseNote>(conn)?)
            })?);
        }
        Ok(out)
    }
}
