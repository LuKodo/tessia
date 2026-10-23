//! `cuv generar` and `cuv barrer`: CUV generation against SISPRO and the sweep
//! of its TXT answers (Go `cmd_cuv.go`, `cmd_cuv_barrer.go`, `internal/cuv`,
//! `internal/adapters/sispro`).
//!
//! * Inputs, from the cargue's artifacts: `<F>.json` (`rips generar`), `<F>.xml`
//!   or the `<F>.xml` entry of `XML/<F>_XML_PDF.zip` (`thefactory descargar`).
//! * Outputs, as artifacts at the root of the cargue: `<F>.txt`, `<F>_ERROR.txt`,
//!   `<F>_RESPUESTA_NO_CORRESPONDE.txt`; the adjusted XML is written back to
//!   `<F>.xml` and into the ZIP.
//! * State: phase `cuv` in `proceso_estado` (DONE / FAILED / REVISION_MANUAL).
//!   Machines split the work through `ProcesoStore` claims.
//! * **Cross-host exclusion per invoice** (replaces Go's `.tessia-cuv` folder
//!   lock): the lease named [`lease_name`] (`cuv:<cargue>:<invoice>`, invoice
//!   normalized, TTL [`LEASE_TTL`]) is held while an invoice is planned, its
//!   XML/ZIP rewritten, sent and stored. `fev ajustar` rewrites the same XML and
//!   ZIP and takes the same lease name, so the two phases never edit one invoice
//!   at the same time, on one machine or on several.
//! * `proceso run` calls [`run_phase`] (own alert) or, to judge a whole run,
//!   [`run_phase_tallied`] with one [`TokenTally`] and [`check_alert`] at the end.

pub mod availability;
pub mod barrer;
pub mod cli;
pub mod generar;
mod jsontree;
pub mod model;
pub mod sispro;
pub mod sweep;
pub mod tls;
pub mod txt;

use std::time::Duration;

use anyhow::{Result, anyhow};

use crate::context::AppContext;
use crate::phases::common::{PhaseReport, PhaseRequest};

pub use availability::{TokenFailureError, TokenPolicy, TokenStats, TokenTally};
pub use cli::Args;
pub use generar::{Execution, MissingRipsJsonError, check_alert, resolve_policy};
pub use sispro::{SisproClient, SisproError, SisproOptions};
pub use txt::txt_has_cuv;

/// How long a per-invoice lease lasts. Go held one `.tessia-cuv` lock (2 h) for
/// the whole folder run; a per-invoice lease only has to outlive the slowest
/// invoice (login tries, a 210 s `CargarFevRips`, two `ConsultarCUV`), and a
/// crashed worker should not block the invoice for hours. It matches the claim
/// TTL (30 min).
pub const LEASE_TTL: Duration = Duration::from_secs(30 * 60);

/// Report reason of an invoice another machine (or `fev ajustar`) is working on.
pub const SKIP_OTHER_MACHINE: &str = "EN_PROCESO_OTRA_MAQUINA";

/// The lease that excludes other workers from one invoice of a cargue:
/// `cuv:<cargue_id>:<invoice>` with the normalized invoice. `fev ajustar` uses
/// the same name.
pub fn lease_name(cargue_id: &str, invoice: &str) -> String {
    format!(
        "cuv:{cargue_id}:{}",
        crate::domain::normalize_invoice(invoice)
    )
}

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    match args.command {
        cli::Command::Generar(generar) => run_generar(ctx, &generar),
        cli::Command::Barrer(barrer) => run_barrer(ctx, &barrer),
    }
}

fn run_generar(ctx: &AppContext, args: &cli::GenerarArgs) -> Result<()> {
    let request = args.common.to_request()?;
    let tally = TokenTally::new();
    let execution = generar::execute(ctx, &request, &tally)?;
    for line in &execution.lines {
        println!("{line}");
    }
    println!(
        "cuv: ready={} skipped={}",
        execution.submitted,
        execution.report.skipped.len()
    );
    println!(
        "cuv: with_cuv={} without_cuv={}",
        execution.with_cuv,
        execution.submitted.saturating_sub(execution.with_cuv)
    );
    println!("{}", execution.report.summary());
    conclude(ctx, execution, Some(&tally)).map(|_| ())
}

fn run_barrer(ctx: &AppContext, args: &cli::BarrerArgs) -> Result<()> {
    let cargue = args
        .cargue
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| anyhow!("falta --cargue <ID> (el nombre del CSV sin extensión)"))?;
    let execution = barrer::execute(ctx, cargue)?;
    for line in &execution.lines {
        println!("{line}");
    }
    match generar::join_errors(&execution.errors) {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// Turns an [`Execution`] into the result of a run: the per-invoice
/// infrastructure errors, then (with a tally) the SISPRO login alert. Outcomes
/// are already stored when either fails.
fn conclude(
    ctx: &AppContext,
    execution: Execution,
    tally: Option<&TokenTally>,
) -> Result<PhaseReport> {
    let failure = generar::join_errors(&execution.errors);
    let alert = tally.and_then(|t| check_alert(ctx, t).err());
    match (failure, alert) {
        (None, None) => Ok(execution.report),
        (Some(err), None) => Err(err),
        (None, Some(alert)) => Err(alert.into()),
        (Some(err), Some(alert)) => Err(anyhow!("{err:#}; {alert}")),
    }
}

/// Runs the phase for the invoices of `req` (called by `proceso run`) as a
/// standalone run: the SISPRO login alert is judged on this call alone.
///
/// Fails with the per-invoice infrastructure errors or the login alert after
/// the outcomes were stored. A requested invoice without RIPS JSON fails with
/// [`MissingRipsJsonError`].
pub fn run_phase(ctx: &AppContext, req: &PhaseRequest) -> Result<PhaseReport> {
    let tally = TokenTally::new();
    let execution = generar::execute(ctx, req, &tally)?;
    conclude(ctx, execution, Some(&tally))
}

/// Like [`run_phase`], but login failures are added to `tally` and the alert is
/// left to the caller (`check_alert` once the whole run is done): `proceso run`
/// calls the phase many times and only the whole run is a meaningful sample.
pub fn run_phase_tallied(
    ctx: &AppContext,
    req: &PhaseRequest,
    tally: &TokenTally,
) -> Result<PhaseReport> {
    let execution = generar::execute(ctx, req, tally)?;
    conclude(ctx, execution, None)
}
