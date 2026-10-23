//! Command line of `fev ajustar` and `fev verificar`.

use clap::{Args as ClapArgs, Subcommand, ValueEnum};

use crate::phases::common::CommonArgs;

/// XML de factura electrónica.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(
        name = "ajustar",
        about = "Ajusta el XML de cada factura del cargue (contrato CUCON o cuota moderadora)",
        long_about = "Edita cada XML del cargue donde está guardado: los <F>.xml sueltos y los .xml \
de <F>.zip, XML/*.zip y ZIP/*.zip (la entrada cambiada se reemplaza y el XML queda al final del ZIP). \
Con --invoices solo las facturas indicadas (<F>.xml y <F>[_XML_PDF].zip).\n\n\
  cucon  NUMERO_CONTRATO 22382 pasa a ser el CUCON (--cucon o minsalud.numero_contrato)\n\
  cuota  asigna el schemeID de PrepaidPayment y quita ANTICIPOS cuando hay un anticipo por legalizar\n\n\
Cada factura queda registrada en la fase fev_ajuste (DONE con los contadores, FAILED si un archivo \
no se pudo leer o escribir) y el total del XML se guarda en factura_valor_xml."
    )]
    Ajustar(AjustarArgs),
    #[command(
        name = "verificar",
        about = "Verifica el total del XML contra el total del convenio",
        long_about = "Compara, por factura, el total del XML (PayableAmount o TaxInclusiveAmount; del primer \
.xml de XML|ZIP/<F>_XML_PDF.zip, si no <F>.xml) con la suma del 'vlr. total' de las líneas del convenio \
(diferencia < 0,01). No escribe archivos: registra la fase fev_valor (DONE; REVISION_MANUAL si difiere; \
FAILED si el XML no tiene valor) y guarda el total del XML en factura_valor_xml. Termina con código 1 \
cuando alguna factura no coincide. Cada ejecución revisa todas las facturas."
    )]
    Verificar(VerificarArgs),
}

/// Ajuste que se aplica al XML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OpName {
    /// NUMERO_CONTRATO 22382 pasa a ser el CUCON
    Cucon,
    /// schemeID de PrepaidPayment y retiro de ANTICIPOS
    Cuota,
}

/// Arguments of the `ajustar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct AjustarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Ajuste a aplicar (obligatorio): cucon o cuota
    #[arg(long, value_enum)]
    pub op: Option<OpName>,
    /// Código del contrato para --op cucon (por defecto minsalud.numero_contrato)
    #[arg(long, value_name = "VALOR")]
    pub cucon: Option<String>,
}

/// Arguments of the `verificar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct VerificarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}
