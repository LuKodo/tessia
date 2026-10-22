//! `proceso run`: the orchestrator that streams every prepared invoice of a
//! cargue through the per-invoice phases, plus `diario` (the previous-day cron
//! entry point, [`diario`]).
//!
//! Port of Go `cmd_proceso*.go`, `proceso_pipeline*.go` and `docs/proceso.md`
//! (catalog PROC-01..17). The pipeline is
//!
//! ```text
//! thefactory -> fev -> (conciliar, only if fev flags VALOR_DIFERENTE) -> derechos
//!            -> rips -> cuv -> soporte -> radicación queue (batch gate)
//! ```
//!
//! Module map: [`cli`]/[`config`] (flags), [`steps`] (order, `--from/--to/--skip`),
//! [`seed`] (step 0), [`pipeline`] (the engine), [`concurrency`] (adaptive
//! pools), [`classify`] (outcome of a phase call), [`runner`] (the seam to the
//! phases), [`supply`] (replacements), [`events`] (text/JSON/panel stream),
//! [`run`] (glue), [`diario`].
//!
//! # Multi-machine behavior (S3)
//!
//! Several machines can run `proceso run` (or `diario`) on the same cargue at
//! the same time and split the work, because every phase claims each
//! (invoice, phase) in `proceso_estado` before working on it
//! ([`ProcesoStore::claim_available`](crate::stores::proceso::ProcesoStore)):
//!
//! * **No cargue-level lease.** A `proceso:<cargue>` lease would make the
//!   second machine wait or fail, which is the opposite of splitting. The only
//!   exclusions are short, one-off steps: `proceso_preparar:<cargue>` (preparing
//!   the cargue; a machine that finds it taken waits and then reuses the
//!   dataset the other prepared) and radicación's own `radicacion:<cargue>`
//!   lease, which makes uploads of one cargue sequential across machines.
//! * **An invoice owned by another machine is not a failure.** A phase that
//!   skips it with `EN_PROCESO_OTRA_MAQUINA` makes this machine stop following
//!   that invoice (the owner carries it on): an `invoice_step` with status
//!   `SKIPPED`, counted in `run_end.skipped`, never `FAILED`, never replaced and
//!   not counted as a drop for the replacement breaker.
//! * **Preparation is not repeated.** A cargue already prepared for its newest
//!   CSV upload is read as it is; re-preparing (Go did it on every run) would
//!   reset the dataset and the `convenio` states under the other machines
//!   (`--repreparar` forces it).
//! * **Radicación waits for the other filer.** If another machine holds the
//!   radicación lease the batch is retried every
//!   [`Tuning::radicacion_retry`](config::Tuning) for up to
//!   [`Tuning::radicacion_wait`](config::Tuning), then reported as `SKIPPED`
//!   (those invoices stay queued in the database for the next run), not as an
//!   error.
//! * Replacement (`--limit N`) only exists in the call that prepared the CSV
//!   (it owns the leftover pool); the other machines just process what is
//!   prepared.
//!
//! # Differences from Go (S1, S3-S6)
//!
//! No dry-run/`--live`/confirmation; the positional argument is a CSV path or a
//! cargue id instead of a load folder; `--nota-tecnica` and `--out` are gone;
//! `radicacion_tiempos` and the timing block belong to radicación (stderr);
//! the dashboard observer, `convenio_progress` and `radicacion_enter` events
//! are not emitted; the adaptive concurrency halves on a transient Postgres
//! conflict that survived the store's retries (serialization failure,
//! deadlock, lost connection) instead of on MySQL lock conflicts (the grow
//! back numbers are Go's: hold 40 finished invoices, +1 per 20 clean ones);
//! the wave-size parameter is gone (always adaptive); the panel repaints on
//! events (no timer thread).
//!
//! # Seam for tests
//!
//! [`runner::PhaseRunner`] holds one closure per step. [`run::execute`] takes
//! it with an [`events::Events`] sink, so the whole command is testable with
//! fake phases over a real database.

pub mod classify;
pub mod cli;
pub mod concurrency;
pub mod config;
pub mod diario;
pub mod events;
pub mod lease;
pub mod pipeline;
pub mod run;
pub mod runner;
pub mod seed;
pub mod steps;
pub mod supply;

use anyhow::Result;

use crate::context::AppContext;

pub use cli::{Args, Command, RunArgs};
pub use config::RunConfig;
pub use events::{Event, Events, Mode};
pub use run::{RunSummary, execute};
pub use runner::PhaseRunner;

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    let Command::Run(args) = args.command;
    let cfg = args.to_config()?;
    let events = Events::stdout(cfg.mode);
    let runner = PhaseRunner::real(&cfg);
    execute(ctx, &cfg, &runner, &events).map(|_| ())
}
