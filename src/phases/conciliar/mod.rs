//! `conciliar`: reconciles `fev_valor` differences against Medicar.
//!
//! Skeleton: the owner of this phase replaces the bodies below.

pub mod catalog;
pub mod cli;
#[cfg(any(test, feature = "test-support"))]
pub mod fake_portal;
pub mod files;
pub mod medicar;
pub mod model;
pub mod plan;
pub mod portal_html;
pub mod rules;

#[cfg(test)]
mod medicar_tests;
#[cfg(test)]
mod plan_tests;
#[cfg(test)]
mod rules_tests;

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
