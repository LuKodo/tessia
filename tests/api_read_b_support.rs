//! Helpers of `tests/api_read_b.rs`: an `ApiState` over a throwaway Postgres,
//! the in-memory bucket and NT, reached only through `ApiState::handle`.
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::Value;
use tessia::api::{ApiState, Request, Response};
use tessia::config::Settings;
use tessia::context::AppContext;
use tessia::db::DbPool;
use tessia::db::test_support::TestDb;
use tessia::jobs::JobRunner;

pub struct Env {
    pub db: TestDb,
    pub state: ApiState,
}

pub fn env() -> Env {
    env_with(Vec::new())
}

/// An environment whose settings hold `pairs` (`TESSIA_*` names).
pub fn env_with(pairs: Vec<(&str, String)>) -> Env {
    let db = TestDb::new();
    let mut ctx = AppContext::for_tests(&db);
    if !pairs.is_empty() {
        let root = ctx.workdir_root.to_string_lossy().into_owned();
        let mut all = vec![("TESSIA_WORKDIR_ROOT", root)];
        all.extend(pairs);
        ctx = ctx.with_settings(Settings::from_pairs(all));
    }
    // The API's own small pool, like the real wiring.
    let pool = DbPool::connect(db.url(), 3).expect("api pool");
    let jobs = Arc::new(JobRunner::new(ctx.clone()));
    let state = ApiState::new(ctx, pool, jobs);
    Env { db, state }
}

impl Env {
    pub fn send(&self, req: Request) -> Response {
        self.state.handle(req)
    }

    pub fn get(&self, path: &str, query: &[(&str, &str)]) -> Response {
        let mut req = Request::new("GET", path);
        for (k, v) in query {
            req = req.with_query(k, v);
        }
        self.send(req)
    }

    pub fn put_json(&self, path: &str, body: &str) -> Response {
        self.send(Request::new("PUT", path).with_body(body))
    }

    pub fn post(&self, path: &str) -> Response {
        self.send(Request::new("POST", path))
    }
}

pub fn json(response: &Response) -> Value {
    serde_json::from_slice(&response.body).unwrap_or_else(|e| {
        panic!(
            "body is not JSON ({e}): {}",
            String::from_utf8_lossy(&response.body)
        )
    })
}

pub fn text(response: &Response) -> String {
    String::from_utf8_lossy(&response.body).into_owned()
}

pub fn error(response: &Response) -> String {
    json(response)["error"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

// ---- seeding ---------------------------------------------------------------

use diesel::RunQueryDsl;
use diesel::sql_types::Text;
use tessia::domain::dataset::Dataset;
use tessia::domain::dataset::test_support::DatasetBuilder;
use tessia::domain::radicacion::{RadState, RadStatus, Step};
use tessia::domain::{Outcome, Phase};
use tessia::stores::dataset::DatasetStore;
use tessia::stores::dataset::test_support::seed_cargue;
use tessia::stores::proceso::{ProcesoStore, SaveRow};
use tessia::stores::radicacion::RadicacionStore;

impl Env {
    /// Registers the upload `load` (hash `hash_char`) and stores the dataset
    /// `build` makes for it, which becomes the load's current one.
    pub fn dataset(
        &self,
        load: &str,
        hash_char: char,
        build: impl FnOnce(DatasetBuilder) -> DatasetBuilder,
    ) -> Dataset {
        let seq = seed_cargue(&self.db.pool(), load, hash_char);
        let ds = build(DatasetBuilder::new(seq).load(load)).build();
        DatasetStore::new(self.db.pool())
            .replace(&ds, false)
            .expect("store dataset");
        ds
    }

    /// Stores a phase outcome and pins its `updated_at` (`2026-09-12 07:MM:00Z`).
    pub fn phase(&self, invoice: &str, phase: Phase, outcome: Outcome, minute: u32) {
        ProcesoStore::new(self.db.pool())
            .save(&SaveRow {
                invoice: invoice.into(),
                phase,
                outcome,
                intentos: 1,
                cargue_id: None,
            })
            .expect("save phase state");
        self.set_time(
            "proceso_estado",
            "num_factura",
            invoice,
            Some(phase.as_str()),
            &format!("2026-09-12 07:{minute:02}:00+00"),
        );
    }

    /// Stores a filing state and pins its `updated_at` (any timestamptz text).
    pub fn filing(&self, invoice: &str, step: Step, status: RadStatus, detail: &str, at: &str) {
        let mut state = RadState::new(invoice, step);
        state.status = status;
        state.last_detail = detail.into();
        RadicacionStore::new(self.db.pool())
            .save(&state)
            .expect("save filing state");
        self.set_time("radicacion_estado", "invoice", invoice, None, at);
    }

    fn set_time(&self, table: &str, key: &str, invoice: &str, phase: Option<&str>, at: &str) {
        let mut sql = format!("UPDATE {table} SET updated_at = $1::timestamptz WHERE {key} = $2");
        if phase.is_some() {
            sql.push_str(" AND fase = $3");
        }
        let mut conn = self.db.conn();
        let query = diesel::sql_query(sql)
            .bind::<Text, _>(at)
            .bind::<Text, _>(invoice);
        match phase {
            Some(p) => query.bind::<Text, _>(p).execute(&mut conn),
            None => query.execute(&mut conn),
        }
        .expect("pin updated_at");
    }
}
