//! Command line of `convenio preparar`.

use clap::Args as ClapArgs;

use crate::phases::common::CommonArgs;

/// Prepara las facturas de un cargue.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(flatten)]
    pub common: CommonArgs,
}
