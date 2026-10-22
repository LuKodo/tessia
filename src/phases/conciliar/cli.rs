//! Command line of `conciliar`.

use clap::Args as ClapArgs;

use crate::phases::common::CommonArgs;

/// Concilia las diferencias de valor entre el XML y el convenio.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(flatten)]
    pub common: CommonArgs,
}
