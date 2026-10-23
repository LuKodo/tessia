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
    #[command(
        name = "generar",
        about = "Genera el JSON RIPS de cada factura (<F>.json, Res. 2275)",
        long_about = "Construye, para cada factura preparada cuyos derechos estén en DONE, el JSON RIPS \
que generaba la pantalla Java EnvioRIPS_Financiera_Extra (salida Gson byte a byte) y lo publica \
como artefacto <F>.json del cargue. Las facturas excluidas, en REVISION_MANUAL o sin derechos \
validados se listan y se omiten. Cada resultado se registra en la fase rips: SIN_SERVICIOS, \
DATOS_INVALIDOS y AFILIADO_NO_COINCIDE quedan en REVISION_MANUAL y solo se reintentan con \
--invoices, igual que las facturas cuyo estado fev_valor no es DONE."
    )]
    Generar(GenerarArgs),
}

/// Arguments of the `generar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct GenerarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Código REPS de 12 dígitos con sede (por defecto company.codigo_prestador + "01")
    #[arg(long = "cod-prestador", value_name = "CODIGO")]
    pub cod_prestador: Option<String>,
}
