//! Command line of `thefactory descargar`.

use clap::{Args as ClapArgs, Subcommand};

use crate::phases::common::CommonArgs;

/// The Factory: XML, PDF y soporte CPScan.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(
        name = "descargar",
        about = "Descarga el XML, PDF y ZIP de cada factura"
    )]
    Descargar(DescargarArgs),
}

/// Arguments of the `descargar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct DescargarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}
