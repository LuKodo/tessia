//! `proceso_estado`: the state of every (invoice, phase) and the claims that
//! let several machines work on the same cargue without colliding.
//!
//! # Model (port of Go `mysqlstore/proceso.go`)
//!
//! * One row per `(num_factura, fase)` with `estado` in
//!   `DONE | FAILED | REVISION_MANUAL | EXCLUDED | EN_PROGRESO`, a Spanish
//!   `motivo`, an `intentos` counter and a `version`.
//! * **Claim** ([`ProcesoStore::start_all`], [`ProcesoStore::claim_available`]):
//!   marks invoices `EN_PROGRESO` with a compare-and-set on the state the
//!   caller loaded (`expected`): an invoice with no row is inserted
//!   (`version 0`); an existing row is updated only if its `estado` *and*
//!   `version` still match. Only one machine can win a given
//!   `(invoice, phase)`. A claim is stamped with `claimed_by`/`claimed_at`.
//! * **Stale takeover**: an `EN_PROGRESO` row whose `claimed_at` is older than
//!   `ClaimOptions::stale_after` (the worker died) can be claimed by another
//!   owner by passing it as `expected = EN_PROGRESO, version = N`; the version
//!   moves on, so the dead worker's late [`ProcesoStore::save_outcome`] is
//!   ignored.
//! * **Outcome** ([`ProcesoStore::save_outcome`]): writes the result only while
//!   the row is still `EN_PROGRESO` at the claimed version; otherwise returns
//!   `false` and writes nothing (a stale result). It bumps `intentos`.
//!
//! ```no_run
//! # use tessia::{db::DbPool, domain::{Phase, Outcome}, stores::proceso::*};
//! # use std::time::Duration;
//! # fn demo(pool: DbPool) -> anyhow::Result<()> {
//! let store = ProcesoStore::new(pool);
//! let invoices = vec!["FEV1".to_string(), "FEV2".to_string()];
//! let loaded = store.load(Phase::Rips, Some(&invoices))?;
//! let items: Vec<ClaimItem> = invoices.iter().map(|i| ClaimItem::from_loaded(i, loaded.get(i))).collect();
//! let opts = ClaimOptions::new("host:42:uuid", Some("Convenio_21102026_21102026"), Duration::from_secs(1800));
//! // Partial claim: take what other machines have not taken.
//! for (invoice, version) in store.claim_available(Phase::Rips, &items, &opts)? {
//!     // ... do the work, then:
//!     let stored = store.save_outcome(Phase::Rips, &invoice, &Outcome::done(), version)?;
//!     assert!(stored, "someone took over our claim");
//! }
//! # Ok(()) }
//! ```

mod schema;

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use diesel::dsl::sql;
use diesel::prelude::*;
use diesel::sql_types::Bool;
use diesel::upsert::excluded;

use super::chunks;
use crate::db::{DbError, DbPool, DbResult};
use crate::domain::{Outcome, Phase, PhaseState};
use schema::proceso_estado as t;

/// One `(invoice, phase)` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcesoState {
    pub invoice: String,
    pub phase: Phase,
    pub state: PhaseState,
    pub motivo: String,
    pub intentos: i32,
    pub version: i64,
    pub cargue_id: Option<String>,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = t)]
struct Row {
    num_factura: String,
    fase: String,
    cargue_id: Option<String>,
    estado: String,
    motivo: String,
    intentos: i32,
    version: i64,
    claimed_by: Option<String>,
    claimed_at: Option<DateTime<Utc>>,
    next_attempt_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<Row> for ProcesoState {
    type Error = DbError;

    fn try_from(row: Row) -> Result<Self, DbError> {
        Ok(ProcesoState {
            invoice: row.num_factura,
            phase: row.fase.parse().map_err(DbError::Other)?,
            state: row.estado.parse().map_err(DbError::Other)?,
            motivo: row.motivo,
            intentos: row.intentos,
            version: row.version,
            cargue_id: row.cargue_id,
            claimed_by: row.claimed_by,
            claimed_at: row.claimed_at,
            next_attempt_at: row.next_attempt_at,
            updated_at: row.updated_at,
        })
    }
}

/// The state a caller loaded for an invoice, used as the compare-and-set
/// precondition of a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expected {
    pub state: PhaseState,
    pub version: i64,
}

/// An invoice to claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimItem {
    pub invoice: String,
    /// `None`: the invoice has no row (claim by insert).
    pub expected: Option<Expected>,
}

impl ClaimItem {
    /// An invoice with no row yet.
    pub fn new(invoice: &str) -> Self {
        Self {
            invoice: invoice.to_owned(),
            expected: None,
        }
    }

    /// From what [`ProcesoStore::load`] returned for the invoice (`None` when
    /// it had no row).
    pub fn from_loaded(invoice: &str, loaded: Option<&ProcesoState>) -> Self {
        Self {
            invoice: invoice.to_owned(),
            expected: loaded.map(|s| Expected {
                state: s.state,
                version: s.version,
            }),
        }
    }
}

/// Who claims, for which cargue, and when another worker's claim is dead.
#[derive(Debug, Clone)]
pub struct ClaimOptions {
    /// Worker identity, e.g. `AppContext::owner`.
    pub owner: String,
    /// Stored in `cargue_id` to list a cargue's invoices later.
    pub cargue_id: Option<String>,
    /// An `EN_PROGRESO` claim older than this may be taken over.
    pub stale_after: Duration,
}

impl ClaimOptions {
    pub fn new(owner: &str, cargue_id: Option<&str>, stale_after: Duration) -> Self {
        Self {
            owner: owner.to_owned(),
            cargue_id: cargue_id.map(str::to_owned),
            stale_after,
        }
    }
}

/// A result to store without a claim (plain upsert).
#[derive(Debug, Clone)]
pub struct SaveRow {
    pub invoice: String,
    pub phase: Phase,
    pub outcome: Outcome,
    pub intentos: i32,
    pub cargue_id: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = t)]
struct NewRow<'a> {
    num_factura: &'a str,
    fase: &'a str,
    cargue_id: Option<&'a str>,
    estado: &'a str,
    motivo: &'a str,
    intentos: i32,
}

impl<'a> NewRow<'a> {
    fn from_save(row: &'a SaveRow) -> Self {
        NewRow {
            num_factura: &row.invoice,
            fase: row.phase.as_str(),
            cargue_id: row.cargue_id.as_deref(),
            estado: row.outcome.state.as_str(),
            motivo: &row.outcome.motivo,
            intentos: row.intentos,
        }
    }
}

/// Predicate: the claim is older than `stale_after` (or has no timestamp).
fn is_stale(stale_after: Duration) -> diesel::expression::SqlLiteral<Bool> {
    // `secs` comes from a Duration (a plain number), never from user text.
    sql::<Bool>(&format!(
        "(claimed_at IS NULL OR claimed_at < now() - make_interval(secs => {:.3}))",
        stale_after.as_secs_f64()
    ))
}

#[derive(Clone)]
pub struct ProcesoStore {
    pool: DbPool,
}

impl ProcesoStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// States of `phase` for `invoices` (every invoice when `None`).
    pub fn load(
        &self,
        phase: Phase,
        invoices: Option<&[String]>,
    ) -> DbResult<HashMap<String, ProcesoState>> {
        let mut out = HashMap::new();
        let mut read = |filter: Option<&[String]>| -> DbResult<()> {
            let rows: Vec<Row> = self.pool.run("load proceso", |conn| {
                let mut query = t::table.filter(t::fase.eq(phase.as_str())).into_boxed();
                if let Some(list) = filter {
                    query = query.filter(t::num_factura.eq_any(list));
                }
                Ok(query.select(Row::as_select()).load(conn)?)
            })?;
            for row in rows {
                let state = ProcesoState::try_from(row)?;
                out.insert(state.invoice.clone(), state);
            }
            Ok(())
        };
        match invoices {
            None => read(None)?,
            Some(list) => {
                for part in chunks(list) {
                    read(Some(part))?;
                }
            }
        }
        Ok(out)
    }

    /// Every state of a cargue (set when the invoice was claimed), optionally
    /// of one phase, ordered by invoice then phase.
    pub fn list_by_cargue(
        &self,
        cargue_id: &str,
        phase: Option<Phase>,
    ) -> DbResult<Vec<ProcesoState>> {
        let rows: Vec<Row> = self.pool.run("list proceso by cargue", |conn| {
            let mut query = t::table.filter(t::cargue_id.eq(cargue_id)).into_boxed();
            if let Some(p) = phase {
                query = query.filter(t::fase.eq(p.as_str()));
            }
            Ok(query
                .order((t::num_factura.asc(), t::fase.asc()))
                .select(Row::as_select())
                .load(conn)?)
        })?;
        rows.into_iter().map(ProcesoState::try_from).collect()
    }

    /// Claims every item or none (Go `StartAll`). Returns `invoice -> version`
    /// of each claim, to pass to [`save_outcome`](Self::save_outcome), or
    /// `None` when any item could not be claimed (nothing is retained).
    pub fn start_all(
        &self,
        phase: Phase,
        items: &[ClaimItem],
        opts: &ClaimOptions,
    ) -> DbResult<Option<HashMap<String, i64>>> {
        let result = self.pool.transaction("claim phase (all)", |conn| {
            let claimed = claim_items(conn, phase, items, opts)?;
            if claimed.len() == items.len() {
                Ok(claimed)
            } else {
                Err(DbError::Query(diesel::result::Error::RollbackTransaction))
            }
        });
        match result {
            Ok(claimed) => Ok(Some(claimed)),
            Err(DbError::Query(diesel::result::Error::RollbackTransaction)) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// Claims every item that can be claimed and skips the rest (the ones
    /// another machine owns or changed). This is what N machines running the
    /// same cargue use to split the work.
    pub fn claim_available(
        &self,
        phase: Phase,
        items: &[ClaimItem],
        opts: &ClaimOptions,
    ) -> DbResult<HashMap<String, i64>> {
        self.pool
            .transaction("claim phase", |conn| claim_items(conn, phase, items, opts))
    }

    /// Stores the result of a claimed invoice. True when written; false when
    /// the row is no longer `EN_PROGRESO` at `expected_version` (the claim was
    /// taken over or the result is stale): nothing is written. Bumps
    /// `intentos` and `version` and clears the claim.
    pub fn save_outcome(
        &self,
        phase: Phase,
        invoice: &str,
        outcome: &Outcome,
        expected_version: i64,
    ) -> DbResult<bool> {
        self.pool.run("save outcome", |conn| {
            let rows = diesel::update(
                t::table.filter(
                    t::num_factura
                        .eq(invoice)
                        .and(t::fase.eq(phase.as_str()))
                        .and(t::estado.eq(PhaseState::EnProgreso.as_str()))
                        .and(t::version.eq(expected_version)),
                ),
            )
            .set((
                t::estado.eq(outcome.state.as_str()),
                t::motivo.eq(&outcome.motivo),
                t::intentos.eq(t::intentos + 1),
                t::version.eq(t::version + 1),
                t::claimed_by.eq(None::<String>),
                t::claimed_at.eq(None::<DateTime<Utc>>),
                t::updated_at.eq(diesel::dsl::now),
            ))
            .execute(conn)?;
            Ok(rows > 0)
        })
    }

    /// Gives claims back (Go `CancelStarted`): each invoice in `versions` that
    /// is still `EN_PROGRESO` at that version returns to its `previous` state
    /// (as loaded before the claim) or is deleted when it had none. Returns how
    /// many were restored; fewer than requested means a claim changed hands.
    pub fn release(
        &self,
        phase: Phase,
        versions: &HashMap<String, i64>,
        previous: &HashMap<String, ProcesoState>,
    ) -> DbResult<usize> {
        self.pool.transaction("release claims", |conn| {
            let mut restored = 0;
            for (invoice, version) in versions {
                let claimed = t::table.filter(
                    t::num_factura
                        .eq(invoice)
                        .and(t::fase.eq(phase.as_str()))
                        .and(t::estado.eq(PhaseState::EnProgreso.as_str()))
                        .and(t::version.eq(*version)),
                );
                restored += match previous.get(invoice) {
                    Some(old) => diesel::update(claimed)
                        .set((
                            t::estado.eq(old.state.as_str()),
                            t::motivo.eq(&old.motivo),
                            t::intentos.eq(old.intentos),
                            t::next_attempt_at.eq(old.next_attempt_at),
                            t::version.eq(t::version + 1),
                            t::claimed_by.eq(None::<String>),
                            t::claimed_at.eq(None::<DateTime<Utc>>),
                            t::updated_at.eq(diesel::dsl::now),
                        ))
                        .execute(conn)?,
                    None => diesel::delete(claimed).execute(conn)?,
                };
            }
            Ok(restored)
        })
    }

    /// Unconditional upsert (Go `Save`): `intentos` is stored as given.
    pub fn save(&self, row: &SaveRow) -> DbResult<()> {
        self.save_all(std::slice::from_ref(row))
    }

    /// Unconditional upsert of many rows, chunked (Go `SaveAll`).
    pub fn save_all(&self, rows: &[SaveRow]) -> DbResult<()> {
        for part in chunks(rows) {
            let values: Vec<NewRow<'_>> = part.iter().map(NewRow::from_save).collect();
            self.pool.run("save proceso", |conn| {
                diesel::insert_into(t::table)
                    .values(&values)
                    .on_conflict((t::num_factura, t::fase))
                    .do_update()
                    .set((
                        t::estado.eq(excluded(t::estado)),
                        t::motivo.eq(excluded(t::motivo)),
                        t::intentos.eq(excluded(t::intentos)),
                        t::cargue_id.eq(diesel::dsl::sql(
                            "COALESCE(EXCLUDED.cargue_id, proceso_estado.cargue_id)",
                        )),
                        t::version.eq(t::version + 1),
                        t::claimed_by.eq(None::<String>),
                        t::claimed_at.eq(None::<DateTime<Utc>>),
                        t::updated_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Records outcomes for a phase continuing the attempts counter from
    /// `prev` (Go `proceso.Record`): `intentos = prev.intentos + 1`.
    pub fn record(
        &self,
        phase: Phase,
        prev: &HashMap<String, ProcesoState>,
        outcomes: &[(String, Outcome)],
        cargue_id: Option<&str>,
    ) -> DbResult<()> {
        let rows: Vec<SaveRow> = outcomes
            .iter()
            .map(|(invoice, outcome)| SaveRow {
                invoice: invoice.clone(),
                phase,
                outcome: outcome.clone(),
                intentos: prev.get(invoice).map_or(0, |s| s.intentos) + 1,
                cargue_id: cargue_id.map(str::to_owned),
            })
            .collect();
        self.save_all(&rows)
    }

    /// Creates the row only when none exists (Go `InsertOutcome`); true when
    /// inserted.
    pub fn insert_outcome(&self, row: &SaveRow) -> DbResult<bool> {
        self.pool.run("insert outcome", |conn| {
            Ok(diesel::insert_into(t::table)
                .values(NewRow::from_save(row))
                .on_conflict_do_nothing()
                .execute(conn)?
                > 0)
        })
    }

    /// Updates a previously loaded row only if its state and version still
    /// match (Go `SaveIfCurrent`); true when written.
    pub fn save_if_current(&self, row: &SaveRow, expected: &ProcesoState) -> DbResult<bool> {
        self.pool.run("save if current", |conn| {
            let rows = diesel::update(
                t::table.filter(
                    t::num_factura
                        .eq(&row.invoice)
                        .and(t::fase.eq(row.phase.as_str()))
                        .and(t::estado.eq(expected.state.as_str()))
                        .and(t::version.eq(expected.version)),
                ),
            )
            .set((
                t::estado.eq(row.outcome.state.as_str()),
                t::motivo.eq(&row.outcome.motivo),
                t::intentos.eq(row.intentos),
                t::version.eq(t::version + 1),
                t::claimed_by.eq(None::<String>),
                t::claimed_at.eq(None::<DateTime<Utc>>),
                t::updated_at.eq(diesel::dsl::now),
            ))
            .execute(conn)?;
            Ok(rows > 0)
        })
    }
}

/// Claims items one by one inside the caller's transaction, in invoice order
/// (a stable order keeps concurrent batches from deadlocking each other).
fn claim_items(
    conn: &mut PgConnection,
    phase: Phase,
    items: &[ClaimItem],
    opts: &ClaimOptions,
) -> DbResult<HashMap<String, i64>> {
    let mut ordered: Vec<&ClaimItem> = items.iter().collect();
    ordered.sort_by(|a, b| a.invoice.cmp(&b.invoice));
    let mut claimed = HashMap::new();
    for item in ordered {
        if let Some(version) = claim_one(conn, phase, item, opts)? {
            claimed.insert(item.invoice.clone(), version);
        }
    }
    Ok(claimed)
}

/// Changes applied by every successful claim of an existing row. `None`
/// fields (no cargue) are left untouched by Diesel.
#[derive(AsChangeset)]
#[diesel(table_name = t)]
struct ClaimChanges<'a> {
    estado: &'a str,
    motivo: &'a str,
    version: i64,
    claimed_by: &'a str,
    cargue_id: Option<&'a str>,
}

fn claim_one(
    conn: &mut PgConnection,
    phase: Phase,
    item: &ClaimItem,
    opts: &ClaimOptions,
) -> DbResult<Option<i64>> {
    let in_progress = PhaseState::EnProgreso.as_str();
    let key = t::num_factura
        .eq(&item.invoice)
        .and(t::fase.eq(phase.as_str()));
    let changes = |version: i64| {
        (
            ClaimChanges {
                estado: in_progress,
                motivo: "",
                version,
                claimed_by: &opts.owner,
                cargue_id: opts.cargue_id.as_deref(),
            },
            t::claimed_at.eq(diesel::dsl::now),
            t::updated_at.eq(diesel::dsl::now),
        )
    };
    let claimed = match item.expected {
        None => {
            let row = (
                t::num_factura.eq(&item.invoice),
                t::fase.eq(phase.as_str()),
                t::cargue_id.eq(opts.cargue_id.as_deref()),
                t::estado.eq(in_progress),
                t::version.eq(0_i64),
                t::claimed_by.eq(Some(opts.owner.as_str())),
                t::claimed_at.eq(diesel::dsl::now),
            );
            let rows = diesel::insert_into(t::table)
                .values(row)
                .on_conflict_do_nothing()
                .execute(conn)?;
            (rows > 0).then_some(0)
        }
        Some(Expected {
            state: PhaseState::EnProgreso,
            version,
        }) => {
            // Takeover: only a claim that has outlived its TTL.
            let target = t::table.filter(
                key.and(t::estado.eq(in_progress))
                    .and(t::version.eq(version))
                    .and(is_stale(opts.stale_after)),
            );
            let rows = diesel::update(target)
                .set(changes(version + 1))
                .execute(conn)?;
            (rows > 0).then_some(version + 1)
        }
        Some(Expected { state, version }) => {
            let target = t::table.filter(
                key.and(t::estado.eq(state.as_str()))
                    .and(t::version.eq(version)),
            );
            let rows = diesel::update(target)
                .set(changes(version + 1))
                .execute(conn)?;
            (rows > 0).then_some(version + 1)
        }
    };
    Ok(claimed)
}

#[cfg(test)]
mod tests;
