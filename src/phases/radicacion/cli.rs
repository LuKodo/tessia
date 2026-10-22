//! Command line of `radicacion run` and `radicacion recheck-step2`.

use clap::{Args as ClapArgs, Subcommand};

use crate::phases::common::CommonArgs;

/// Radicación en Mutual Ser.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(name = "run", about = "Radica las facturas en Mutual Ser")]
    Run(RunArgs),
    #[command(name = "recheck-step2", about = "Revisa de nuevo el paso 2 (CUV)")]
    RecheckStep2(RecheckStep2Args),
}

/// Arguments of the `run` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct RunArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}

/// Arguments of the `recheck-step2` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct RecheckStep2Args {
    #[command(flatten)]
    pub common: CommonArgs,
}
