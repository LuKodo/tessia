//! `soporte generar`: CRC support ZIP generation (PDF rendering).
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

/// Runs the phase for the invoices of `req` (called by `proceso run`).
pub fn run_phase(_ctx: &AppContext, _req: &PhaseRequest) -> Result<PhaseReport> {
    bail!("not implemented yet")
}
