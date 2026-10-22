//! Command line of `fev ajustar` and `fev verificar`.

use clap::{Args as ClapArgs, Subcommand};

use crate::phases::common::CommonArgs;

/// XML de factura electrónica.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(name = "ajustar", about = "Ajusta el XML de cada factura")]
    Ajustar(AjustarArgs),
    #[command(
        name = "verificar",
        about = "Verifica el total del XML contra el convenio"
    )]
    Verificar(VerificarArgs),
}

/// Arguments of the `ajustar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct AjustarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}

/// Arguments of the `verificar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct VerificarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}
