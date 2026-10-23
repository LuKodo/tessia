//! The in-process dashboard API (Go: `cmd/tessia/serve*.go` + `internal/dashboard`).
//!
//! No socket: the desktop shell turns each custom-protocol request into a
//! [`Request`] and calls [`ApiState::handle`]. The contract is in
//! `odd/reference/ui-api-spec.md`.

pub mod cache;
pub mod csv;
pub mod errors;
pub mod guard;
pub mod http;
pub mod read;
pub mod request;
pub mod router;
pub mod routes;

use std::sync::Arc;

pub use errors::{ApiError, ApiResult};
pub use request::{Request, Response};

use crate::context::AppContext;
use crate::db::DbPool;
use crate::jobs::JobRunner;

/// Everything a handler needs.
pub struct ApiState {
    pub ctx: AppContext,
    /// The API's own small pool, so a running job cannot starve the dashboard.
    pub pool: DbPool,
    pub jobs: Arc<JobRunner>,
    /// Random hex per process; the UI detects a restart when it changes.
    pub instance: String,
    /// App origins trusted by the same-origin guard; the shell registers its own.
    pub origins: guard::OriginGuard,
    /// Shared 2.5 s answer cache of the heavy GET routes.
    pub cache: cache::Cache,
}

impl ApiState {
    pub fn new(ctx: AppContext, pool: DbPool, jobs: Arc<JobRunner>) -> Self {
        let instance = jobs.instance_id().to_string();
        Self {
            ctx,
            pool,
            jobs,
            instance,
            origins: guard::OriginGuard::new(),
            cache: cache::Cache::new(),
        }
    }

    /// Trusts one more app origin on the state-changing routes (the origin the
    /// shell's custom protocol is served from).
    pub fn allow_origin(&self, origin: &str) {
        self.origins.allow(origin);
    }

    /// Routes one request. Never panics on a bad request.
    pub fn handle(&self, req: Request) -> Response {
        router::dispatch(self, req)
    }
}
