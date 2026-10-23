//! Command line of `convenio preparar`.

use clap::Args as ClapArgs;

/// Prepara las facturas de un cargue: lee el CSV, lo cruza con la Nota Técnica
/// (Redis), aplica las reglas por factura, verifica MIPRES (EVENTO NO PBS) y
/// guarda el resultado.
#[derive(Debug, Clone, Default, ClapArgs)]
pub struct Args {
    /// CSV de convenio local (se registra como cargue y se sube al bucket) o
    /// id de un cargue que ya existe (se lee su CSV del bucket)
    #[arg(value_name = "CSV_O_CARGUE")]
    pub source: Option<String>,

    /// Id de un cargue existente (alternativa al argumento posicional)
    #[arg(long, value_name = "ID", conflicts_with = "source")]
    pub cargue: Option<String>,

    /// Monto máximo por factura en COP, p. ej. 100000 (por defecto el de los
    /// parámetros guardados o convenio.max_invoice_amount)
    #[arg(long, value_name = "COP")]
    pub monto_maximo: Option<String>,

    /// Prepara solo estas facturas (separadas por coma); fuerza el subplan
    #[arg(long, value_name = "FACTURAS", value_delimiter = ',')]
    pub invoices: Vec<String>,

    /// Prepara hasta N facturas pendientes (0 = todas); las excluidas o en
    /// revisión manual no cuentan y las ya terminadas no consumen el límite
    #[arg(
        long,
        value_name = "N",
        default_value_t = 0,
        allow_negative_numbers = true
    )]
    pub limit: i64,
}
