//! Command line of `derechos validar`.

use clap::{Args as ClapArgs, Subcommand};

use crate::phases::common::CommonArgs;

/// Derechos de afiliados.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(name = "validar", about = "Valida los derechos de los afiliados")]
    Validar(ValidarArgs),
}

/// Arguments of the `validar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct ValidarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}
