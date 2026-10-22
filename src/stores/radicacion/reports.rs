//! Read models of the dashboard reports (manual-review report and filed
//! invoices) that join the dataset, `proceso_estado` and `radicacion_estado`.
//! Read-only: nothing here writes.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};

use crate::db::{DbPool, DbResult};

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
}
