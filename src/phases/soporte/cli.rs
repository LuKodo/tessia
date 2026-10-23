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
        about = "Genera el ZIP de soporte CRC de cada factura",
        long_about = "Para cada factura del cargue (o de --invoices) genera CRC/CRC_<nit>_<F>.zip: un PDF con la(s) página(s) de soporte seguida(s) de <F>.pdf.\n\
El soporte sale de CPSCAN/<F>.cpscan.json y <F>.support.<ext> (los deja `thefactory descargar`); una factura sin ellos falla con CPSCAN_NO_DESCARGADO y se reintenta. \
Si CPScan no tiene soporte se arma el soporte manual (rama=MANUAL: constancia de entrega con la firma del paciente, generada desde el nombre si no existe), salvo depósitos obligatorios y sede 002 (SOPORTE_OBLIGATORIO_NO_DISPONIBLE). \
La constancia lista todas las líneas de la factura con el SSC de la primera; las de otro SSC se reportan. \
Los ZIP existentes se conservan (YA_EXISTE) salvo con --regenerar. Cada factura queda en proceso_estado (fase soporte)."
    )]
    Generar(GenerarArgs),
}

/// Arguments of the `generar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct GenerarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Reconstruye el soporte aunque su ZIP exista y lo reemplaza (conserva el anterior si falla); solo en esta llamada explícita
    #[arg(long)]
    pub regenerar: bool,
}
