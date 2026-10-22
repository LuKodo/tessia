//! Command line of `rips generar`.

use clap::{Args as ClapArgs, Subcommand};

use crate::phases::common::CommonArgs;

/// RIPS.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(name = "generar", about = "Genera el JSON RIPS de cada factura")]
    Generar(GenerarArgs),
}

/// Arguments of the `generar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct GenerarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}
