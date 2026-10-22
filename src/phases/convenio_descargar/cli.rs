//! Command line of `convenio descargar`.

use clap::Args as ClapArgs;

use crate::phases::common::CommonArgs;

/// Descarga el CSV de convenio de Medicar.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(flatten)]
    pub common: CommonArgs,
}
