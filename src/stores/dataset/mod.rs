//! The prepared convenio dataset in Postgres (port of Go
//! `mysqlstore/dataset_store.go`, `value_tables.go` and the `cargue_actual`
//! pointer of `cargue_store.go`).
//!
//! Tables (migration `2026-10-23-000000_dataset_tables`), all keyed by
//! `cargues.seq` (the "cargue" number of every method):
//!
//! | table | content |
//! |---|---|
//! | `dataset_preparacion` | header: load, `nt_version`, `max_amount`, `automatic_subplans`, `prepared_format`, stats, skipped lines, `prepared_at` |
//! | `dataset_factura` | every invoice (state `PREPARADA`/`EXCLUIDA`/`REVISION_MANUAL`, position, `version`, MIPRES ids, reconciliation) |
//! | `dataset_linea` | lines of the prepared invoices (`data` JSONB = the whole [`Line`](crate::domain::dataset::Line)) |
//! | `dataset_actual` | per load, the cargue readers resolve ([`DatasetStore::current_cargue`]) |
//!
//! # Semantics (Go)
//!
//! * **Replace** ([`DatasetStore::replace`]) stores a dataset in one
//!   transaction and makes its cargue the *current* one of its load. With
//!   `merge` it keeps the stored invoices the new dataset does not name (see
//!   [`DatasetStore::replace_touching`]). An invoice stored again gets
//!   `version + 1`, so a reader holding the old version can never write.
//! * **Update** ([`DatasetStore::update_invoices`]) rewrites the lines, MIPRES
//!   ids and reconciliation of prepared invoices, keeping their position.
//!   Rows are locked in name order and every expected version is checked
//!   before anything is written: any mismatch is
//!   [`DatasetError::VersionConflict`] and nothing changes. Each updated
//!   invoice's version grows by one. `convenio preparar` never conflicts (it
//!   replaces); `conciliar` does when its invoice was re-prepared meanwhile.
//! * **Subset reads** transfer only the named invoices (names are normalized
//!   with [`normalize_invoice`](crate::domain::normalize_invoice); the header
//!   is always the whole dataset's).
//! * **Agreement total** ([`DatasetStore::agreement_total`]) is never stored:
//!   the sum of the line totals of the invoice in the newest *current* cargue
//!   that holds it as `PREPARADA` (ties: highest cargue seq; an invoice stored
//!   twice in one cargue under the same normalized name: the later one).
//! * `ErrCargueSuperseded` is raised by
//!   [`CarguesStore::ingest`](crate::stores::cargues::CarguesStore::ingest),
//!   not here: `replace` works on whatever cargue it is given.
//!
//! ```no_run
//! # use tessia::{stores::dataset::DatasetStore, domain::dataset::test_support::*};
//! # fn demo(store: DatasetStore, cargue: i64) -> anyhow::Result<()> {
//! let ds = DatasetBuilder::new(cargue)
//!     .invoice(InvoiceBuilder::new("EVS1").line("100.00"))
//!     .build();
//! store.replace(&ds, false)?;
//! let mut read = store.read_invoices(cargue, Some(&["evs-1".to_string()]))?;
//! read[0].lines[0].quantity = "2".into();
//! let expected = read[0].version;
//! store.update_invoices(cargue, &[(read.remove(0), expected)])?;
//! # Ok(()) }
//! ```

pub mod dashboard;
mod error;
mod rows;
mod schema;
pub mod summary;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};
use rust_decimal::Decimal;
use serde_json::Value;

use super::chunks;
use crate::db::{DbPool, with_retry};
use crate::domain::dataset::{
    Dataset, Excluded, Invoice, InvoiceAudit, InvoiceState, ManualReview, Preparation,
    subset_names, sum_line_totals,
};
use crate::domain::normalize_invoice;
use crate::util::decimal::round2;
pub use error::{DatasetError, DatasetResult};
use rows::{FacturaDraft, NewFactura, NewLinea, NewPrep, PrepRow};
use schema::{
    dataset_actual as act, dataset_factura as fac, dataset_linea as lin,
    dataset_preparacion as prep,
};
use summary::{InvoiceSummary, summarize};

/// Rows per multi-row `INSERT` (well under Postgres' 65,535 bind limit).
const INSERT_CHUNK: usize = 1000;

#[derive(QueryableByName)]
struct LoadKey {
    #[diesel(sql_type = Text)]
    id: String,
}

#[derive(QueryableByName)]
struct CurrentSeq {
    #[diesel(sql_type = Nullable<BigInt>)]
    seq: Option<i64>,
}

#[derive(QueryableByName)]
struct TotalRow {
    #[diesel(sql_type = Nullable<Text>)]
    total_value: Option<String>,
}

/// Store of the prepared dataset. Cheap to clone; get it from
/// [`AppContext::dataset`](crate::context::AppContext::dataset).
#[derive(Clone)]
pub struct DatasetStore {
    pool: DbPool,
}

impl DatasetStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    // ---- writes -------------------------------------------------------

    /// Stores `ds` under `ds.cargue` in one transaction and makes that cargue
    /// the current one of its load.
    ///
    /// * `merge = false`: the stored dataset of the cargue (if any) is
    ///   replaced as a whole.
    /// * `merge = true` (Go `convenio preparar --invoices`): every invoice
    ///   `ds` names (prepared, excluded or manual review) replaces the stored
    ///   one; the other stored invoices keep their position, `ds`'s invoices
    ///   are appended; the stored `stats`/`skipped_lines` win and
    ///   `prepared_format` is the OR of both. A cargue with no stored dataset,
    ///   or stored for another `load.id`, is replaced as a whole.
    ///
    /// `ds.preparation.prepared_at == None` means "now" (database clock).
    /// Invoice versions: new invoices start at `0`; an invoice that was stored
    /// before gets its old version `+ 1`. `Invoice::version` in `ds` is ignored.
    pub fn replace(&self, ds: &Dataset, merge: bool) -> DatasetResult<()> {
        let touched = if merge { ds.all_names() } else { Vec::new() };
        self.replace_touching(ds, &touched)
    }

    /// [`replace`](Self::replace) with an explicit list of touched invoices
    /// (Go `touched`): empty replaces the whole dataset of the cargue;
    /// otherwise the merge described there, dropping `touched` (also the names
    /// that `ds` does not contain, e.g. requested invoices absent from the CSV).
    pub fn replace_touching(&self, ds: &Dataset, touched: &[String]) -> DatasetResult<()> {
        let (drafts, lines) = encode_dataset(ds)?;
        let touched = subset_names(touched);
        let cargue = ds.cargue;
        self.in_tx("replace dataset", false, |conn| {
            let load_key = load_key_of(conn, cargue)?;
            let stored: Option<(String, Value, Value, bool)> = prep::table
                .filter(prep::cargue_seq.eq(cargue))
                .select((
                    prep::load_id,
                    prep::stats,
                    prep::skipped_lines,
                    prep::prepared_format,
                ))
                .for_update()
                .first(conn)
                .optional()?;
            let merge = !touched.is_empty()
                && stored
                    .as_ref()
                    .is_some_and(|s| s.0 == ds.preparation.load.id);

            let prev = drop_invoices(conn, cargue, merge.then_some(touched.as_slice()))?;
            let base = if merge {
                next_position(conn, cargue)?
            } else {
                0
            };

            let prepared_at = match ds.preparation.prepared_at {
                Some(at) => at,
                None => db_now(conn)?,
            };
            let header = match (&stored, merge) {
                (Some((_, stats, skipped, fmt)), true) => NewPrep::new(
                    cargue,
                    &ds.preparation,
                    prepared_at,
                    stats.clone(),
                    skipped.clone(),
                    *fmt || ds.preparation.prepared_format,
                )?,
                _ => NewPrep::new(
                    cargue,
                    &ds.preparation,
                    prepared_at,
                    rows::to_json(&ds.preparation.stats)?,
                    rows::to_json(&ds.preparation.skipped_lines)?,
                    ds.preparation.prepared_format,
                )?,
            };
            upsert_header(conn, &header)?;

            let facturas: Vec<NewFactura> = drafts
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let version = prev
                        .get(&normalize_invoice(&d.name))
                        .map_or(0, |old| old + 1);
                    d.clone_row(cargue, base + i as i32, version)
                })
                .collect();
            for chunk in facturas.chunks(INSERT_CHUNK) {
                diesel::insert_into(fac::table)
                    .values(chunk)
                    .execute(conn)
                    .map_err(unique_as_invalid)?;
            }
            for chunk in lines.chunks(INSERT_CHUNK * 2) {
                diesel::insert_into(lin::table)
                    .values(chunk)
                    .execute(conn)?;
            }
            point_to_cargue(conn, cargue, &load_key)
        })
    }

    /// Replaces the lines, MIPRES ids and reconciliation of stored prepared
    /// invoices, keeping their position. Each item is the invoice plus the
    /// `version` it was read with ([`Invoice::version`] from a read).
    ///
    /// All-or-nothing: the rows are locked in name order, then checked in name
    /// order; the first missing/not-prepared invoice is
    /// [`DatasetError::InvoiceNotFound`], the first stale one
    /// [`DatasetError::VersionConflict`], and nothing is written. Otherwise each
    /// version grows by one. An invoice named twice is `Invalid`.
    pub fn update_invoices(&self, cargue: i64, items: &[(Invoice, i64)]) -> DatasetResult<()> {
        let mut sorted: Vec<&(Invoice, i64)> = items.iter().collect();
        sorted.sort_by(|a, b| a.0.invoice.cmp(&b.0.invoice));
        if let Some(pair) = sorted.windows(2).find(|w| w[0].0.invoice == w[1].0.invoice) {
            return Err(DatasetError::Invalid(format!(
                "la factura {} aparece dos veces",
                pair[0].0.invoice
            )));
        }
        let mut audits = Vec::with_capacity(sorted.len());
        let mut summaries = Vec::with_capacity(sorted.len());
        let mut lines: Vec<NewLinea> = Vec::new();
        for (invoice, _) in &sorted {
            audits.push(rows::encode_audit(invoice)?);
            summaries.push(summarize(&invoice.lines));
            lines.extend(rows::line_rows(cargue, invoice)?);
        }
        self.in_tx("update dataset invoices", false, |conn| {
            let names: Vec<&str> = sorted.iter().map(|(i, _)| i.invoice.as_str()).collect();
            let mut found: HashMap<String, (String, i64)> = HashMap::new();
            for chunk in chunks(&names) {
                let locked: Vec<(String, String, i64)> = fac::table
                    .filter(fac::cargue_seq.eq(cargue).and(fac::factura.eq_any(chunk)))
                    .order(fac::factura.asc())
                    .select((fac::factura, fac::estado, fac::version))
                    .for_update()
                    .load(conn)?;
                found.extend(locked.into_iter().map(|(n, s, v)| (n, (s, v))));
            }
            for (invoice, expected) in &sorted {
                match found.get(&invoice.invoice) {
                    Some((state, stored))
                        if InvoiceState::parse(state) == Some(InvoiceState::Prepared) =>
                    {
                        if stored != expected {
                            return Err(DatasetError::VersionConflict {
                                invoice: invoice.invoice.clone(),
                                stored: *stored,
                                expected: *expected,
                            });
                        }
                    }
                    _ => {
                        return Err(DatasetError::InvoiceNotFound {
                            cargue,
                            invoice: invoice.invoice.clone(),
                        });
                    }
                }
            }
            for chunk in chunks(&names) {
                diesel::delete(
                    lin::table.filter(lin::cargue_seq.eq(cargue).and(lin::factura.eq_any(chunk))),
                )
                .execute(conn)?;
            }
            for (((invoice, _), audit), summary) in sorted.iter().zip(&audits).zip(&summaries) {
                diesel::update(
                    fac::table.filter(
                        fac::cargue_seq
                            .eq(cargue)
                            .and(fac::factura.eq(&invoice.invoice)),
                    ),
                )
                .set((
                    fac::id_mipres.eq(&invoice.id_mipres),
                    fac::conciliacion.eq(audit),
                    fac::fecha_factura_txt.eq(&summary.date),
                    fac::ssc.eq(&summary.ssc),
                    fac::subplan.eq(&summary.subplan),
                    fac::autorizacion.eq(&summary.authorization),
                    fac::valor_convenio.eq(summary.agreement_value),
                    fac::version.eq(fac::version + 1),
                    fac::updated_at.eq(diesel::dsl::now),
                ))
                .execute(conn)?;
            }
            for chunk in lines.chunks(INSERT_CHUNK * 2) {
                diesel::insert_into(lin::table)
                    .values(chunk)
                    .execute(conn)?;
            }
            Ok(())
        })
    }

    /// Writes only the header of the cargue (insert or replace of the whole
    /// header row; invoices are untouched). Use it to pin `nt_version` or save
    /// stats/skipped lines before or without a dataset. It does not move the
    /// current-cargue pointer (only [`replace`](Self::replace) does).
    pub fn save_preparation(&self, cargue: i64, preparation: &Preparation) -> DatasetResult<()> {
        let stats = rows::to_json(&preparation.stats)?;
        let skipped = rows::to_json(&preparation.skipped_lines)?;
        self.in_tx("save dataset preparation", false, |conn| {
            load_key_of(conn, cargue)?;
            let at = match preparation.prepared_at {
                Some(at) => at,
                None => db_now(conn)?,
            };
            let header = NewPrep::new(
                cargue,
                preparation,
                at,
                stats.clone(),
                skipped.clone(),
                preparation.prepared_format,
            )?;
            upsert_header(conn, &header)
        })
    }

    // ---- reads --------------------------------------------------------

    /// The header of the cargue's dataset, or `None` when it has none.
    pub fn preparation(&self, cargue: i64) -> DatasetResult<Option<Preparation>> {
        let row = self.pool.run("read dataset preparation", |conn| {
            Ok(prep::table
                .filter(prep::cargue_seq.eq(cargue))
                .select(PrepRow::as_select())
                .first(conn)
                .optional()?)
        })?;
        row.map(PrepRow::into_preparation).transpose()
    }

    /// The dataset of the cargue on one snapshot (a concurrent `replace` is
    /// seen whole or not at all). `subset`:
    ///
    /// * `None`: everything;
    /// * `Some(names)`: only those invoices (normalized, repeated names read
    ///   once, unknown names absent) in `invoices`/`excluded`/`manual_review`,
    ///   in stored order, with lines and versions; the header is the whole
    ///   dataset's. `Some(&[])` reads the header only.
    ///
    /// [`DatasetError::NotFound`] when the cargue has no dataset.
    pub fn read(&self, cargue: i64, subset: Option<&[String]>) -> DatasetResult<Dataset> {
        let names = subset.map(subset_names);
        self.in_tx("read dataset", true, |conn| {
            let header: PrepRow = prep::table
                .filter(prep::cargue_seq.eq(cargue))
                .select(PrepRow::as_select())
                .first(conn)
                .optional()?
                .ok_or(DatasetError::NotFound(cargue))?;
            let mut ds = Dataset {
                cargue,
                preparation: header.into_preparation()?,
                ..Dataset::default()
            };
            if names.as_ref().is_some_and(Vec::is_empty) {
                return Ok(ds);
            }
            let facturas = read_facturas(conn, cargue, names.as_deref())?;
            let mut lines = read_lines(conn, cargue, names.as_deref())?;
            for f in facturas {
                match InvoiceState::parse(&f.estado) {
                    Some(InvoiceState::Prepared) => {
                        let conciliacion = f
                            .conciliacion
                            .map(serde_json::from_value::<InvoiceAudit>)
                            .transpose()
                            .map_err(|e| {
                                DatasetError::Invalid(format!(
                                    "conciliación ilegible de {}: {e}",
                                    f.factura
                                ))
                            })?;
                        ds.invoices.push(Invoice {
                            lines: lines.remove(&f.factura).unwrap_or_default(),
                            invoice: f.factura,
                            conciliacion,
                            id_mipres: f.id_mipres,
                            version: f.version,
                        });
                    }
                    Some(InvoiceState::Excluded) => ds.excluded.push(Excluded {
                        invoice: f.factura,
                        reason: f.motivo,
                    }),
                    Some(InvoiceState::ManualReview) => ds.manual_review.push(ManualReview {
                        invoice: f.factura,
                        code: f.codigo,
                        reason: f.motivo,
                    }),
                    None => {
                        return Err(DatasetError::Invalid(format!(
                            "la factura {} tiene el estado desconocido {:?}",
                            f.factura, f.estado
                        )));
                    }
                }
            }
            Ok(ds)
        })
    }

    /// The prepared invoices of the cargue (all, or the `subset`), with lines
    /// and `version` ready for [`update_invoices`](Self::update_invoices).
    /// Shortcut for `read(..)?.invoices`.
    pub fn read_invoices(
        &self,
        cargue: i64,
        subset: Option<&[String]>,
    ) -> DatasetResult<Vec<Invoice>> {
        Ok(self.read(cargue, subset)?.invoices)
    }

    /// [`read`](Self::read) of the *current* cargue of the load `load_id`
    /// (`None` when the load has no cargue or its current one has no dataset).
    pub fn read_current(
        &self,
        load_id: &str,
        subset: Option<&[String]>,
    ) -> DatasetResult<Option<Dataset>> {
        let Some(cargue) = self.current_cargue(load_id)? else {
            return Ok(None);
        };
        match self.read(cargue, subset) {
            Ok(ds) => Ok(Some(ds)),
            Err(DatasetError::NotFound(_)) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// Names of the prepared invoices of the cargue, in stored order.
    pub fn invoice_names(&self, cargue: i64) -> DatasetResult<Vec<String>> {
        self.names_with_state(cargue, InvoiceState::Prepared)
    }

    /// Names of the invoices of the cargue in `state`, in stored order.
    pub fn names_with_state(&self, cargue: i64, state: InvoiceState) -> DatasetResult<Vec<String>> {
        Ok(self.pool.run("dataset invoice names", |conn| {
            Ok(fac::table
                .filter(
                    fac::cargue_seq
                        .eq(cargue)
                        .and(fac::estado.eq(state.as_str())),
                )
                .order(fac::orden.asc())
                .select(fac::factura)
                .load(conn)?)
        })?)
    }

    /// The cargue readers resolve for the load `load_id` (a cargue id such as
    /// `Convenio_21102026_21102026`): the pointer [`replace`](Self::replace)
    /// moved, else the newest upload; `None` when the load has no cargue.
    pub fn current_cargue(&self, load_id: &str) -> DatasetResult<Option<i64>> {
        Ok(self.pool.run("current dataset cargue", |conn| {
            Ok(diesel::sql_query(
                "SELECT COALESCE(
                    (SELECT a.cargue_seq FROM dataset_actual a WHERE a.load_id = $1),
                    (SELECT MAX(n.seq) FROM cargues n WHERE n.id = $1)) AS seq",
            )
            .bind::<Text, _>(load_id)
            .get_result::<CurrentSeq>(conn)?
            .seq)
        })?)
    }

    /// Agreement total of an invoice (Go `ValueTables.AgreementTotal`): the sum
    /// of the line totals of the invoice in the newest current cargue where it
    /// is `PREPARADA`. `None` when no current cargue holds it as prepared
    /// (excluded, in manual review, or unknown); a prepared invoice without
    /// lines totals `0`. A line total that cannot be read fails with
    /// [`DatasetError::Invalid`] (the radicación check then blocks the invoice
    /// as unavailable). The result has two decimals.
    pub fn agreement_total(&self, invoice: &str) -> DatasetResult<Option<Decimal>> {
        let key = normalize_invoice(invoice);
        let rows = self.pool.run("dataset agreement total", |conn| {
            Ok(diesel::sql_query(AGREEMENT_TOTAL_SQL)
                .bind::<Text, _>(&key)
                .load::<TotalRow>(conn)?)
        })?;
        if rows.is_empty() {
            return Ok(None);
        }
        let total = sum_line_totals(rows.iter().filter_map(|r| r.total_value.as_deref()))
            .map_err(|e| DatasetError::Invalid(format!("factura {key}: {e}")))?;
        Ok(Some(round2(total)))
    }

    // ---- plumbing -----------------------------------------------------

    /// Runs `unit` in a transaction (read-only repeatable-read when
    /// `snapshot`). Transient database failures re-run the whole unit; any
    /// other [`DatasetError`] (conflicts included) rolls back and is returned.
    fn in_tx<T>(
        &self,
        op: &str,
        snapshot: bool,
        mut unit: impl FnMut(&mut PgConnection) -> DatasetResult<T>,
    ) -> DatasetResult<T> {
        with_retry(op, || {
            let mut conn = self.pool.get()?;
            let result = if snapshot {
                conn.build_transaction()
                    .repeatable_read()
                    .read_only()
                    .run::<T, DatasetError, _>(|c| unit(c))
            } else {
                conn.transaction::<T, DatasetError, _>(|c| unit(c))
            };
            match result {
                Ok(value) => Ok(Ok(value)),
                Err(DatasetError::Db(e)) => Err(e),
                Err(other) => Ok(Err(other)),
            }
        })?
    }
}

/// Newest current cargue of the invoice that holds it as prepared; one row per
/// line (a single NULL row when the prepared invoice has no lines).
const AGREEMENT_TOTAL_SQL: &str = "SELECT l.total_value
    FROM (SELECT f.cargue_seq, f.factura
            FROM dataset_factura f JOIN cargues g ON g.seq = f.cargue_seq
            WHERE f.factura_norm = $1 AND f.estado = 'PREPARADA'
              AND g.seq = COALESCE(
                    (SELECT a.cargue_seq FROM dataset_actual a WHERE a.load_id = g.id),
                    (SELECT MAX(n.seq) FROM cargues n WHERE n.id = g.id))
            ORDER BY f.cargue_seq DESC, f.orden DESC LIMIT 1) c
    LEFT JOIN dataset_linea l ON l.cargue_seq = c.cargue_seq AND l.factura = c.factura";

impl FacturaDraft {
    fn clone_row(&self, cargue: i64, orden: i32, version: i64) -> NewFactura {
        FacturaDraft {
            name: self.name.clone(),
            state: self.state,
            code: self.code.clone(),
            reason: self.reason.clone(),
            mipres: self.mipres.clone(),
            audit: self.audit.clone(),
            summary: self.summary.clone(),
        }
        .into_row(cargue, orden, version)
    }
}

/// Every invoice of `ds` in the order `read` must return them (prepared, then
/// excluded, then manual review) and the lines of the prepared ones.
fn encode_dataset(ds: &Dataset) -> DatasetResult<(Vec<FacturaDraft>, Vec<NewLinea>)> {
    let mut seen = std::collections::HashSet::new();
    let mut drafts = Vec::new();
    let mut lines = Vec::new();
    let mut check = |name: &str| -> DatasetResult<()> {
        if seen.insert(name.to_owned()) {
            Ok(())
        } else {
            Err(DatasetError::Invalid(format!(
                "el dataset nombra dos veces la factura {name}"
            )))
        }
    };
    for inv in &ds.invoices {
        check(&inv.invoice)?;
        lines.extend(rows::line_rows(ds.cargue, inv)?);
        drafts.push(FacturaDraft {
            name: inv.invoice.clone(),
            state: InvoiceState::Prepared,
            code: String::new(),
            reason: String::new(),
            mipres: inv.id_mipres.clone(),
            audit: rows::encode_audit(inv)?,
            summary: summarize(&inv.lines),
        });
    }
    for e in &ds.excluded {
        check(&e.invoice)?;
        drafts.push(FacturaDraft {
            name: e.invoice.clone(),
            state: InvoiceState::Excluded,
            code: String::new(),
            reason: e.reason.clone(),
            mipres: String::new(),
            audit: None,
            summary: InvoiceSummary::default(),
        });
    }
    for m in &ds.manual_review {
        check(&m.invoice)?;
        drafts.push(FacturaDraft {
            name: m.invoice.clone(),
            state: InvoiceState::ManualReview,
            code: m.code.clone(),
            reason: m.reason.clone(),
            mipres: String::new(),
            audit: None,
            summary: InvoiceSummary::default(),
        });
    }
    Ok((drafts, lines))
}

fn unique_as_invalid(err: diesel::result::Error) -> DatasetError {
    use diesel::result::{DatabaseErrorKind, Error};
    match err {
        Error::DatabaseError(DatabaseErrorKind::UniqueViolation, _) => DatasetError::Invalid(
            "una factura ya está guardada y no está en la lista de facturas a reemplazar".into(),
        ),
        other => other.into(),
    }
}

/// The load key (`cargues.id`) of a cargue; `CargueNotFound` when none.
fn load_key_of(conn: &mut PgConnection, cargue: i64) -> DatasetResult<String> {
    diesel::sql_query("SELECT id FROM cargues WHERE seq = $1")
        .bind::<BigInt, _>(cargue)
        .get_result::<LoadKey>(conn)
        .optional()?
        .map(|k| k.id)
        .ok_or(DatasetError::CargueNotFound(cargue))
}

/// Deletes (and returns the versions of, by normalized name) the invoices of
/// the cargue: the named ones, or all when `names` is `None`. Rows are locked
/// first so concurrent preparations queue.
fn drop_invoices(
    conn: &mut PgConnection,
    cargue: i64,
    names: Option<&[String]>,
) -> DatasetResult<HashMap<String, i64>> {
    let mut prev = HashMap::new();
    match names {
        None => {
            let all: Vec<(String, i64)> = fac::table
                .filter(fac::cargue_seq.eq(cargue))
                .select((fac::factura_norm, fac::version))
                .for_update()
                .load(conn)?;
            prev.extend(all);
            diesel::delete(fac::table.filter(fac::cargue_seq.eq(cargue))).execute(conn)?;
        }
        Some(names) => {
            for chunk in chunks(names) {
                let found: Vec<(String, i64)> = fac::table
                    .filter(
                        fac::cargue_seq
                            .eq(cargue)
                            .and(fac::factura_norm.eq_any(chunk)),
                    )
                    .select((fac::factura_norm, fac::version))
                    .for_update()
                    .load(conn)?;
                prev.extend(found);
                diesel::delete(
                    fac::table.filter(
                        fac::cargue_seq
                            .eq(cargue)
                            .and(fac::factura_norm.eq_any(chunk)),
                    ),
                )
                .execute(conn)?;
            }
        }
    }
    Ok(prev)
}

/// The database clock (`now()`, the start of the transaction).
fn db_now(conn: &mut PgConnection) -> DatasetResult<DateTime<Utc>> {
    Ok(
        diesel::select(diesel::dsl::sql::<diesel::sql_types::Timestamptz>("now()"))
            .get_result(conn)?,
    )
}

fn next_position(conn: &mut PgConnection, cargue: i64) -> DatasetResult<i32> {
    use diesel::dsl::max;
    let top: Option<i32> = fac::table
        .filter(fac::cargue_seq.eq(cargue))
        .select(max(fac::orden))
        .first(conn)?;
    Ok(top.map_or(0, |v| v + 1))
}

fn upsert_header(conn: &mut PgConnection, header: &NewPrep) -> DatasetResult<()> {
    diesel::insert_into(prep::table)
        .values(header)
        .on_conflict(prep::cargue_seq)
        .do_update()
        .set((header, prep::updated_at.eq(diesel::dsl::now)))
        .execute(conn)?;
    Ok(())
}

/// Makes `cargue` the current cargue of its load; the last statement of the
/// transaction so the pointer row is held the shortest.
fn point_to_cargue(conn: &mut PgConnection, cargue: i64, load_key: &str) -> DatasetResult<()> {
    diesel::insert_into(act::table)
        .values((act::load_id.eq(load_key), act::cargue_seq.eq(cargue)))
        .on_conflict(act::load_id)
        .do_update()
        .set((
            act::cargue_seq.eq(cargue),
            act::updated_at.eq(diesel::dsl::now),
        ))
        .execute(conn)?;
    Ok(())
}

#[derive(Queryable)]
struct FacturaRead {
    factura: String,
    estado: String,
    codigo: String,
    motivo: String,
    id_mipres: String,
    conciliacion: Option<Value>,
    version: i64,
}

/// Invoice rows in stored order: all, or the named ones.
fn read_facturas(
    conn: &mut PgConnection,
    cargue: i64,
    names: Option<&[String]>,
) -> DatasetResult<Vec<FacturaRead>> {
    let cols = (
        fac::factura,
        fac::estado,
        fac::codigo,
        fac::motivo,
        fac::id_mipres,
        fac::conciliacion,
        fac::version,
    );
    let Some(names) = names else {
        return Ok(fac::table
            .filter(fac::cargue_seq.eq(cargue))
            .order(fac::orden.asc())
            .select(cols)
            .load(conn)?);
    };
    let mut out: Vec<(i32, FacturaRead)> = Vec::new();
    for chunk in chunks(names) {
        let part: Vec<(i32, FacturaRead)> = fac::table
            .filter(fac::cargue_seq.eq(cargue).and(fac::factura.eq_any(chunk)))
            .select((fac::orden, cols))
            .load(conn)?;
        out.extend(part);
    }
    out.sort_by_key(|(orden, _)| *orden);
    Ok(out.into_iter().map(|(_, f)| f).collect())
}

/// Lines by invoice, in line order: of every invoice, or of the named ones.
fn read_lines(
    conn: &mut PgConnection,
    cargue: i64,
    names: Option<&[String]>,
) -> DatasetResult<HashMap<String, Vec<crate::domain::dataset::Line>>> {
    let mut out: HashMap<String, Vec<crate::domain::dataset::Line>> = HashMap::new();
    let mut take = |rows: Vec<(String, Value)>| -> DatasetResult<()> {
        for (factura, data) in rows {
            out.entry(factura)
                .or_default()
                .push(rows::decode_line(data)?);
        }
        Ok(())
    };
    match names {
        None => take(
            lin::table
                .filter(lin::cargue_seq.eq(cargue))
                .order((lin::factura.asc(), lin::orden.asc()))
                .select((lin::factura, lin::data))
                .load(conn)?,
        )?,
        Some(names) => {
            for chunk in chunks(names) {
                take(
                    lin::table
                        .filter(lin::cargue_seq.eq(cargue).and(lin::factura.eq_any(chunk)))
                        .order((lin::factura.asc(), lin::orden.asc()))
                        .select((lin::factura, lin::data))
                        .load(conn)?,
                )?;
            }
        }
    }
    Ok(out)
}
