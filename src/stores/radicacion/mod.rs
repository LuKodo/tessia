//! `radicacion_estado`: Mutual Ser filing pipeline state, one row per
//! normalized invoice (see [`crate::domain::radicacion`]).

mod reports;
mod schema;

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use diesel::prelude::*;

use super::chunks;
use crate::db::{DbError, DbPool, DbResult};
use crate::domain::normalize_invoice;
use crate::domain::radicacion::{RadState, RadStatus, Step};
pub use reports::{CargueRef, DatasetEntry, FiledInvoice, LineFacts, PhaseNote, ReportStore};
use schema::radicacion_estado as t;

#[derive(Queryable, Selectable)]
#[diesel(table_name = t)]
struct Row {
    invoice: String,
    folder: String,
    step: i16,
    status: String,
    upload_id: String,
    sent_at: Option<DateTime<Utc>>,
    sends: i32,
    polls: i32,
    step1_reloaded: bool,
    reload_for_step2: bool,
    next_attempt_at: Option<DateTime<Utc>>,
    last_detail: Option<String>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<Row> for RadState {
    type Error = DbError;

    fn try_from(row: Row) -> Result<Self, DbError> {
        Ok(RadState {
            invoice: row.invoice,
            folder: row.folder,
            step: Step::from_i16(row.step).ok_or_else(|| {
                DbError::other(format!("paso de radicación inválido: {}", row.step))
            })?,
            status: row.status.parse::<RadStatus>().map_err(DbError::Other)?,
            upload_id: row.upload_id,
            sent_at: row.sent_at,
            sends: row.sends,
            polls: row.polls,
            step1_reloaded: row.step1_reloaded,
            reload_for_step2: row.reload_for_step2,
            next_attempt_at: row.next_attempt_at,
            last_detail: row.last_detail.unwrap_or_default(),
            updated_at: Some(row.updated_at),
        })
    }
}

#[derive(Clone)]
pub struct RadicacionStore {
    pool: DbPool,
}

impl RadicacionStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Stored states of `invoices` (as given; use normalized keys). Missing
    /// invoices are absent from the map.
    pub fn load(&self, invoices: &[String]) -> DbResult<HashMap<String, RadState>> {
        let mut out = HashMap::new();
        for part in chunks(invoices) {
            let rows: Vec<Row> = self.pool.run("load radicacion", |conn| {
                Ok(t::table
                    .filter(t::invoice.eq_any(part))
                    .select(Row::as_select())
                    .load(conn)?)
            })?;
            for row in rows {
                let state = RadState::try_from(row)?;
                out.insert(state.invoice.clone(), state);
            }
        }
        Ok(out)
    }

    /// Inserts or replaces the whole state of the invoice.
    pub fn save(&self, state: &RadState) -> DbResult<()> {
        self.pool.run("save radicacion", |conn| {
            let values = || {
                (
                    t::invoice.eq(&state.invoice),
                    t::folder.eq(&state.folder),
                    t::step.eq(state.step.as_i16()),
                    t::status.eq(state.status.as_str()),
                    t::upload_id.eq(&state.upload_id),
                    t::sent_at.eq(state.sent_at),
                    t::sends.eq(state.sends),
                    t::polls.eq(state.polls),
                    t::step1_reloaded.eq(state.step1_reloaded),
                    t::reload_for_step2.eq(state.reload_for_step2),
                    t::next_attempt_at.eq(state.next_attempt_at),
                    t::last_detail.eq(Some(state.last_detail.as_str())),
                )
            };
            diesel::insert_into(t::table)
                .values(values())
                .on_conflict(t::invoice)
                .do_update()
                .set((values(), t::updated_at.eq(diesel::dsl::now)))
                .execute(conn)?;
            Ok(())
        })
    }

    /// The subset of `invoices` (normalized, deduplicated) that the pipeline
    /// considers finished (see [`RadState::is_finished`]): read in chunks, no
    /// per-invoice round trips. Unknown and unfinished invoices are absent.
    pub fn finished(&self, invoices: &[String]) -> DbResult<HashSet<String>> {
        let mut seen = HashSet::new();
        let normalized: Vec<String> = invoices
            .iter()
            .map(|i| normalize_invoice(i))
            .filter(|i| !i.is_empty() && seen.insert(i.clone()))
            .collect();
        Ok(self
            .load(&normalized)?
            .into_values()
            .filter(RadState::is_finished)
            .map(|s| s.invoice)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::TestDb;

    fn state(invoice: &str, step: Step, status: RadStatus) -> RadState {
        let mut s = RadState::new(invoice, step);
        s.status = status;
        s
    }

    #[test]
    fn save_then_load_round_trips_every_field() {
        let db = TestDb::new();
        let store = RadicacionStore::new(db.pool());
        let mut s = state("FEV1", Step::Rips, RadStatus::Sent);
        s.folder = "/w/x".into();
        s.upload_id = "U1".into();
        s.sent_at = Some(Utc::now());
        s.sends = 2;
        s.polls = 5;
        s.step1_reloaded = true;
        s.reload_for_step2 = true;
        s.next_attempt_at = Some(Utc::now());
        s.last_detail = "detalle".into();
        store.save(&s).expect("save");
        let loaded = store.load(&["FEV1".into(), "NOPE".into()]).expect("load");
        assert_eq!(loaded.len(), 1);
        let got = &loaded["FEV1"];
        assert!(got.updated_at.is_some());
        let mut expected = s.clone();
        expected.updated_at = got.updated_at;
        // timestamptz keeps microseconds
        expected.sent_at = got.sent_at;
        expected.next_attempt_at = got.next_attempt_at;
        assert_eq!(got, &expected);
        assert!(
            (got.sent_at.expect("sent") - s.sent_at.expect("sent"))
                .num_milliseconds()
                .abs()
                < 2
        );
    }

    #[test]
    fn save_replaces_the_state() {
        let db = TestDb::new();
        let store = RadicacionStore::new(db.pool());
        store
            .save(&state("FEV1", Step::InvoiceZip, RadStatus::Pending))
            .expect("first");
        store
            .save(&state("FEV1", Step::Cuv, RadStatus::Confirmed))
            .expect("second");
        let got = &store.load(&["FEV1".into()]).expect("load")["FEV1"];
        assert_eq!((got.step, got.status), (Step::Cuv, RadStatus::Confirmed));
    }

    #[test]
    fn finished_normalizes_dedups_and_ignores_retryable_states() {
        let db = TestDb::new();
        let store = RadicacionStore::new(db.pool());
        store
            .save(&state("FEV1", Step::Crc, RadStatus::Confirmed))
            .expect("done");
        store
            .save(&state("FEV2", Step::Cuv, RadStatus::CreditNote))
            .expect("credit");
        store
            .save(&state("FEV3", Step::Cuv, RadStatus::Blocked))
            .expect("blocked");
        store
            .save(&state("FEV4", Step::Crc, RadStatus::Sent))
            .expect("sent");
        let asked: Vec<String> = ["fev-1", "FEV1", "FEV2", "FEV3", "FEV4", "FEV5", ""]
            .map(String::from)
            .to_vec();
        let finished = store.finished(&asked).expect("finished");
        let mut got: Vec<&str> = finished.iter().map(String::as_str).collect();
        got.sort();
        assert_eq!(got, ["FEV1", "FEV2"]);
    }

    #[test]
    fn large_lookups_are_chunked() {
        let db = TestDb::new();
        let store = RadicacionStore::new(db.pool());
        store
            .save(&state("FEV1", Step::Crc, RadStatus::Confirmed))
            .expect("done");
        let mut asked: Vec<String> = (0..1000).map(|i| format!("ZZZ{i}")).collect();
        asked.push("FEV1".into());
        assert_eq!(store.finished(&asked).expect("finished").len(), 1);
    }
}
