//! Shared helpers of the `api_*` integration tests (included with
//! `#[path = "api_support.rs"] mod support;`).
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::Value;
use tessia::api::{ApiState, Request, Response};
use tessia::config::Settings;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::jobs::JobRunner;
use tessia::nt::{MemoryNt, NtCatalog};
use tessia::storage::{MemoryStore, ObjectStore};

/// An API over a throwaway database, an in-memory bucket and an in-memory NT.
pub struct Harness {
    pub db: TestDb,
    pub state: ApiState,
    pub objects: Arc<MemoryStore>,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness {
    pub fn new() -> Self {
        Self::build(MemoryNt::new(), |settings| settings)
    }

    pub fn with_nt(nt: MemoryNt) -> Self {
        Self::build(nt, |settings| settings)
    }

    pub fn with_settings(f: impl FnOnce(Settings) -> Settings) -> Self {
        Self::build(MemoryNt::new(), f)
    }

    /// Replaces parts of the context (a failing NT or bucket, settings).
    pub fn with_context(f: impl FnOnce(AppContext) -> AppContext) -> Self {
        Self::assemble(f)
    }

    fn build(nt: MemoryNt, settings: impl FnOnce(Settings) -> Settings) -> Self {
        Self::assemble(|ctx| {
            let configured = settings(ctx.settings.clone());
            let nt: Arc<dyn NtCatalog> = Arc::new(nt);
            ctx.with_nt(nt).with_settings(configured)
        })
    }

    fn assemble(f: impl FnOnce(AppContext) -> AppContext) -> Self {
        let db = TestDb::new();
        let objects = Arc::new(MemoryStore::new());
        let bucket: Arc<dyn ObjectStore> = objects.clone();
        let ctx = f(AppContext::for_tests(&db).with_objects(bucket));
        let state = ApiState::new(
            ctx.clone(),
            db.pool(),
            Arc::new(JobRunner::with_history_dir(ctx, None)),
        );
        Self { db, state, objects }
    }

    pub fn send(&self, req: Request) -> Response {
        self.state.handle(req)
    }

    pub fn get(&self, path: &str) -> Response {
        self.send(Request::new("GET", path))
    }
}

pub fn json(resp: &Response) -> Value {
    serde_json::from_slice(&resp.body).unwrap_or_else(|err| {
        panic!(
            "body is not JSON ({err}): {}",
            String::from_utf8_lossy(&resp.body)
        )
    })
}

pub fn error_of(resp: &Response) -> String {
    json(resp)["error"].as_str().unwrap_or_default().to_owned()
}
