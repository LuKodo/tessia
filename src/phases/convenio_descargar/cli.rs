//! Command line of `convenio descargar`.

use clap::Args as ClapArgs;

/// Descarga el CSV de convenio de Medicar y lo publica como cargue.
#[derive(Debug, Clone, Default, ClapArgs)]
#[command(
    override_usage = "facturacion convenio descargar (--desde AAAA-MM-DD --hasta AAAA-MM-DD | --actual)",
    long_about = "Descarga el informe de convenio de Medicar y lo publica como artefacto del cargue \
        Convenio_ddMMyyyy_ddMMyyyy (origen medicar). Necesita medicar.base_url, medicar.products_url, \
        medicar.identifier, medicar.username y medicar.password; medicar.ca_file es opcional. \
        Si otra máquina ya está descargando el mismo rango, el comando falla sin tocar nada."
)]
pub struct Args {
    /// Fecha inicial AAAA-MM-DD (hora local)
    #[arg(long)]
    pub desde: Option<String>,
    /// Fecha final AAAA-MM-DD (hora local)
    #[arg(long)]
    pub hasta: Option<String>,
    /// Del primer día del mes actual hasta hoy (no se combina con --desde/--hasta)
    #[arg(long)]
    pub actual: bool,
}
