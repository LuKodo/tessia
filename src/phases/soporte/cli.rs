//! Command line of `soporte generar`.

use clap::{Args as ClapArgs, Subcommand};

use crate::phases::common::CommonArgs;

/// Soporte CRC.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(
        name = "generar",
        about = "Genera el ZIP de soporte CRC de cada factura"
    )]
    Generar(GenerarArgs),
}

/// Arguments of the `generar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct GenerarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}
