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
        all.extend(pairs.into_iter().map(|(k, v)| (k, v)));
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
