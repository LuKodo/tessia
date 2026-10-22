//! `fev ajustar` and `fev verificar`: invoice XML adjustment and XML-vs-convenio value check.
//!
//! Skeleton: the owner of this phase replaces the bodies below.

pub mod cli;

use anyhow::{Result, bail};

use crate::context::AppContext;
use crate::phases::common::{PhaseReport, PhaseRequest};

pub use cli::Args;

/// Entry point of the command line.
pub fn run(_ctx: &AppContext, _args: Args) -> Result<()> {
    bail!("not implemented yet")
}

/// `fev ajustar` as a phase (`Phase::FevAjuste`), called by `proceso run`.
pub fn run_ajustar_phase(_ctx: &AppContext, _req: &PhaseRequest) -> Result<PhaseReport> {
    bail!("not implemented yet")
}

/// `fev verificar` as a phase (`Phase::FevValor`), called by `proceso run`.
pub fn run_verificar_phase(_ctx: &AppContext, _req: &PhaseRequest) -> Result<PhaseReport> {
    bail!("not implemented yet")
}
