//! The desktop window: the existing web UI embedded in the executable, served
//! through a custom protocol, with the API running in-process (Go: the Wails
//! shell in `cmd/tessia/desktop_*.go`). No TCP listener, like Wails.
//!
//! - [`assets`]: the embedded UI and the static-serving rules.
//! - [`protocol`]: request translation, the API worker pool, the app origin.
//! - [`downloads`]: where `Content-Disposition: attachment` answers are saved.
//! - `window` (feature `desktop`): the tao window and the wry webview.
//!
//! The pure parts compile without the `desktop` feature, so a headless build
//! (a server running the daily cron) needs no GTK/WebKit libraries.

pub mod assets;
pub mod protocol;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::api::ApiState;
use crate::context::AppContext;
use crate::db::DbPool;
use crate::jobs::JobRunner;

/// Window title.
pub const TITLE: &str = "Tessia";

/// Connections of the API's own pool (the jobs use the process pool), a little
/// above the API workers so a burst of requests never waits for a connection.
pub const API_POOL_SIZE: u32 = protocol::API_WORKERS as u32 + 2;

/// How long closing the window waits for API requests already running.
const CLOSE_WAIT: Duration = Duration::from_secs(3);

/// Opens the window and blocks until it is closed. Startup order (as Go): the
/// embedded UI check, then the stores and the job runner, then the window.
/// Closing the window releases the backend and ends the process.
#[cfg(feature = "desktop")]
pub fn run(ctx: AppContext) -> Result<()> {
    assets::require_index(&assets::EmbeddedUi)?;
    let _backend = Backend::open(ctx, Arc::new(assets::EmbeddedUi))?;
    anyhow::bail!("the window is not implemented yet")
}

/// A build without the window (`--no-default-features`).
#[cfg(not(feature = "desktop"))]
pub fn run(_ctx: AppContext) -> Result<()> {
    anyhow::bail!("this build has no desktop window (compiled without the `desktop` feature)")
}

/// The in-process backend of the window: the API state behind the protocol
/// bridge. Built without a window so it is testable.
pub struct Backend {
    bridge: Arc<protocol::Bridge>,
    state: Arc<ApiState>,
}

impl Backend {
    /// Opens the API's own pool and the job runner on `ctx`.
    pub fn open(ctx: AppContext, assets: Arc<dyn assets::AssetSource>) -> Result<Self> {
        let url = ctx.settings.require_one("database.url")?;
        let pool =
            DbPool::connect(url, API_POOL_SIZE).context("no se pudo abrir la base de datos")?;
        let jobs = Arc::new(JobRunner::new(ctx.clone()));
        Ok(Self::from_state(ApiState::new(ctx, pool, jobs), assets))
    }

    /// A backend over an existing state (tests).
    pub fn from_state(state: ApiState, assets: Arc<dyn assets::AssetSource>) -> Self {
        let state = Arc::new(state);
        let api_state = Arc::clone(&state);
        let bridge = protocol::Bridge::new(
            assets,
            Arc::new(move |request| api_state.handle(request)),
            protocol::app_origin(),
        );
        Self {
            bridge: Arc::new(bridge),
            state,
        }
    }

    pub fn bridge(&self) -> Arc<protocol::Bridge> {
        Arc::clone(&self.bridge)
    }

    /// Stops answering requests and releases the stores: no new API work is
    /// accepted, requests already running get a few seconds, then the state
    /// (the API pool and the job runner) is dropped. The running job's
    /// cancellation belongs to `JobRunner` (see the note below).
    pub fn close(self) {
        self.bridge.close(CLOSE_WAIT);
        // `JobRunner` has no shutdown yet (the jobs unit owns it). When it does,
        // cancel and wait for the running job here, before `state` is dropped.
        drop(self.state);
    }
}
