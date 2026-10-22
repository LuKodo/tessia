//! `convenio preparar`: reads a convenio CSV, crosses it with the Nota Técnica
//! (Redis), applies the per-invoice rules, checks MIPRES for EVENTO NO PBS and
//! stores the dataset and the `convenio` phase state of every invoice.
//!
//! Ported from Go `cmd_convenio.go` and `internal/convenio`. Adaptations
//! (S1, S5, S6): no dry-run or confirmation; the NT is the Redis collection
//! (`nt_catalog` documents the field mapping and the duplicate rules, and the
//! version used is pinned in the stored preparation so `conciliar` re-reads
//! the same one); `--nota-tecnica` and `--out` are gone; the CSV is a local
//! file (registered as a cargue and uploaded to the bucket) or an existing
//! cargue (read back from the bucket); the dataset goes to the dataset store.
//!
//! Module map: `csv` (reader), `amount` (number helpers), `catalog` +
//! `nt_catalog` (NT lookup), `subplan`, `prepare` (rule engine, pure),
//! `mipres` (check logic) + `mipres_client` (HTTP), `source` (CSV origin),
//! `run` (orchestration), `summary` (console lines).

pub mod amount;
pub mod catalog;
pub mod cli;
pub mod csv;
pub mod mipres;
pub mod mipres_client;
pub mod nt_catalog;
pub mod prepare;
#[cfg(test)]
mod prepare_tests;
pub mod run;
pub mod source;
pub mod subplan;
pub mod summary;

use anyhow::{Result, bail};

use crate::context::AppContext;
use crate::phases::common::{PhaseReport, PhaseRequest};

pub use cli::Args;
pub use run::{Plan, PrepareRequest, commit_preparation, plan_preparation};

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    if args.limit < 0 {
        bail!("--limit must be >= 0");
    }
    let source = match (&args.source, &args.cargue) {
        (Some(source), None) | (None, Some(source)) => source.clone(),
        _ => bail!("uso: facturacion convenio preparar [opciones] <csv|cargue>"),
    };
    let request = PrepareRequest {
        source,
        max_amount: args.monto_maximo,
        only: args.invoices,
        limit: usize::try_from(args.limit).unwrap_or_default(),
    };
    let plan = plan_preparation(ctx, &request, None)?;
    for line in summary_of(&plan) {
        println!("{line}");
    }
    commit_preparation(ctx, &plan)?;
    println!(
        "{}",
        summary::saved_line(plan.result.invoices.len(), &plan.load.id)
    );
    Ok(())
}

/// The console summary of a plan.
pub fn summary_of(plan: &Plan) -> Vec<String> {
    summary::summary_lines(&summary::SummaryInput {
        csv_name: &plan.load.name,
        load: &plan.load,
        nt_version: &plan.nt_version,
        nt_entries: plan.nt_entries,
        lines_read: plan.csv.lines.len(),
        lines_skipped: plan.csv.skipped,
        result: &plan.result,
    })
}

/// Runs the phase for the invoices of `req` (called by `proceso run`): the
/// cargue `req.cargue_id` must exist; `req.invoices` restricts (and forces)
/// the preparation like `--invoices`. The report lists the stored outcomes.
pub fn run_phase(ctx: &AppContext, req: &PhaseRequest) -> Result<PhaseReport> {
    let request = PrepareRequest {
        source: req.cargue_id.clone(),
        only: req.invoices.clone().unwrap_or_default(),
        ..PrepareRequest::default()
    };
    let plan = plan_preparation(ctx, &request, None)?;
    commit_preparation(ctx, &plan)?;
    let mut report = PhaseReport::default();
    for (invoice, outcome) in run::outcomes(&plan.result) {
        report.push(&invoice, outcome);
    }
    Ok(report)
}
