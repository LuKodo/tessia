//! `soporte generar`: builds `CRC/CRC_<nit>_<F>.zip`, the delivery support of
//! each invoice (support page(s) followed by the invoice PDF).
//!
//! Layout:
//!
//! * `input`: the per-invoice input (`SoporteInput`) and the Go normalization rules;
//! * `cpscan`: reader of the `CPSCAN/*` sidecar left by `thefactory descargar`;
//! * `decide`: result codes and the branch decision;
//! * `generator`: assembles the support PDF (no I/O);
//! * `render`: pure-Rust PDF pages (formula image, delivery page, signature) and merge;
//! * `assets`: shared signature files and logo (bucket artifacts of `_shared`);
//! * `zipfile`: the ZIP container;
//! * `run`: claims, workers and state recording.

pub mod assets;
pub mod cli;
pub mod cpscan;
pub mod decide;
pub mod generator;
pub mod input;
pub mod render;
pub mod run;
pub mod zipfile;

use anyhow::{Result, bail};

use crate::context::AppContext;
use crate::phases::common::{PhaseReport, PhaseRequest};

pub use cli::Args;
pub use input::{SoporteInput, SoporteRow};
pub use run::{Options, SoporteRun, run_inputs};

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    let cli::Command::Generar(generar) = args.command;
    let req = generar.common.to_request()?;
    let options = Options {
        regenerate: generar.regenerar,
    };
    let inputs = dataset_inputs(ctx, &req.cargue_id)?;
    let result = run_inputs(ctx, &req, options, inputs)?;
    for line in &result.lines {
        println!("{line}");
    }
    println!("{}", result.summary_line());
    if !result.errors.is_empty() {
        bail!(
            "soporte: {} factura(s) con error de infraestructura: {}",
            result.errors.len(),
            result.errors.join("; ")
        );
    }
    Ok(())
}

/// Runs the phase for the invoices of `req` (called by `proceso run`; it never regenerates).
pub fn run_phase(ctx: &AppContext, req: &PhaseRequest) -> Result<PhaseReport> {
    let inputs = dataset_inputs(ctx, &req.cargue_id)?;
    Ok(run_inputs(ctx, req, Options::default(), inputs)?.report)
}

/// The dataset lines of the cargue, per invoice (Phase B adapter).
fn dataset_inputs(_ctx: &AppContext, _cargue_id: &str) -> Result<Vec<SoporteInput>> {
    bail!("not implemented yet: the dataset store adapter lands in phase B")
}
