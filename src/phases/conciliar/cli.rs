//! Command line of `conciliar`.

use clap::Args as ClapArgs;

use crate::phases::common::CommonArgs;

/// Reconstruye las líneas del convenio de las facturas cuyo valor XML difiere
/// del convenio (líneas del XML + fecha de la fórmula en Medicar).
///
/// Estados: CONCILIADA (las líneas suman el total del XML: se actualiza la
/// factura, fev_valor queda DONE y se borran <F>.json/<F>.txt/<F>_ERROR.txt
/// obsoletos; luego se ejecutan rips generar y cuv generar para esas facturas),
/// DIFERENCIA_PERSISTE (REVISION_MANUAL), BLOQUEADA_TARIFA (EXCLUDED: un precio
/// del XML supera la nota técnica) y BLOQUEADA_MONTO (EXCLUDED: el total
/// reconstruido supera el tope por factura). Los resultados quedan en la fase
/// `conciliacion`. Medicar solo se consulta para las CONCILIADA
/// (medicar.base_url, identifier, username, password). Sin --invoices se
/// omiten las facturas ya EXCLUDED o REVISION_MANUAL en esta fase.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(flatten)]
    pub common: CommonArgs,
}
