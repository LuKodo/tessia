//! Command line of `proceso run`.

use clap::{Args as ClapArgs, Subcommand};

use crate::phases::common::CommonArgs;

/// Proceso completo.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(name = "run", about = "Ejecuta todas las fases de un cargue")]
    Run(RunArgs),
}

/// Arguments of the `run` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct RunArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}
