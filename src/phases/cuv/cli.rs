//! Command line of `cuv generar` and `cuv barrer`.

use clap::{Args as ClapArgs, Subcommand};

use crate::phases::common::CommonArgs;

/// Código Único de Validación.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(
        name = "generar",
        about = "Envía el XML de la FEV y el JSON RIPS a SISPRO y guarda el CUV",
        long_about = "Para cada <F>.json del cargue (de `rips generar`) con su XML <F>.xml (o el XML de \
XML/<F>_XML_PDF.zip, de `thefactory descargar`): ajusta el XML como prepararXmlMinisterioEvento, \
envía {rips, xmlFevFile} a CargarFevRips y guarda la respuesta como en Java: <F>.txt (200/201), \
<F>_ERROR.txt (400) o <F>_RESPUESTA_NO_CORRESPONDE.txt, después del barrido RVG18. Las facturas \
cuyo <F>.txt ya tiene CUV se omiten. El resultado queda en el estado de la fase cuv: DONE, FAILED \
(se reintenta) o REVISION_MANUAL (rechazos; solo se reenvía con --invoices). Sin --invoices se omiten \
las facturas en REVISION_MANUAL y las que tienen estado rips distinto de DONE. El login a SISPRO se \
reintenta (3 intentos con espera creciente), nunca CargarFevRips. Si más del porcentaje configurado \
de las facturas enviadas no pudo iniciar sesión (por defecto 50 %, mínimo 4 enviadas) el comando \
falla al final, ya guardados los resultados. Varias máquinas pueden ejecutarlo a la vez: cada una \
toma las facturas que nadie más tiene. --workers son las facturas en vuelo (por defecto 40, máximo 40); \
un limitador adaptativo (8/20/40) regula las peticiones a SISPRO.\n\nSe necesitan minsalud.numero_contrato, \
company.codigo_prestador y minsalud.base_url, tipo_doc, numero_doc, clave y nit; opcionalmente \
minsalud.ca_file o minsalud.tls_pin_sha256 para el certificado del servidor."
    )]
    Generar(GenerarArgs),
    #[command(
        name = "barrer",
        about = "Corrige los TXT de CUV del cargue (RVG18 -> CUV existente + ProcesoId)",
        long_about = "Reescribe cada TXT del cargue que sea un objeto JSON como \
Vista_ActualizarCUVMinisterioLocal: ResultState=true, NumFactura desde RutaArchivos y, para un \
resultado RVG18, el CUV existente pasa a CodigoUnicoValidacion, el ProcesoId sale de ConsultarCUV \
y ResultadosValidacion queda vacío. Los demás TXT no se tocan; los que Java no podía procesar se \
informan como ERROR y no se escriben. Un <F>.txt reescrito que ahora tiene CUV queda DONE en la \
fase cuv."
    )]
    Barrer(BarrerArgs),
}

/// Arguments of the `generar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct GenerarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}

/// Arguments of the `barrer` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct BarrerArgs {
    /// Cargue a barrer (el nombre del CSV sin extensión, p. ej. Convenio_21102026_21102026)
    #[arg(long, value_name = "ID")]
    pub cargue: Option<String>,
}
