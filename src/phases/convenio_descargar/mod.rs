//! `convenio descargar`: downloads the convenio CSV from Medicar into the bucket.
//!
//! Skeleton: the owner of this phase replaces the bodies below.

pub mod cli;

use anyhow::{Result, bail};

use crate::context::AppContext;

pub use cli::Args;

/// Entry point of the command line.
pub fn run(_ctx: &AppContext, _args: Args) -> Result<()> {
    bail!("not implemented yet")
}
