//! Command line of `cuv generar` and `cuv barrer`.

use clap::{Args as ClapArgs, Subcommand};

use crate::phases::common::CommonArgs;

/// Código Único de Validación.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(name = "generar", about = "Obtiene el CUV de cada factura")]
    Generar(GenerarArgs),
    #[command(name = "barrer", about = "Barre y reconcilia los CUV")]
    Barrer(BarrerArgs),
}

/// Arguments of the `generar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct GenerarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}

/// Arguments of the `barrer` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct BarrerArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}
