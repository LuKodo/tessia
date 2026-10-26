//! What the RIPS builder needs from the convenio dataset. The dataset adapter
//! (`selection.rs`) fills these structs; everything else in this module is
//! pure over them.

/// One prepared convenio line (Go `convenio.Line`, only the fields RIPS uses).
/// All values are the strings of the CSV / the enrichment, untouched: the
/// builder reproduces Java's parsing of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RipsLine {
    /// `source_line`: row of the original CSV, quoted in `DATOS_INVALIDOS` details.
    pub source_line: i32,
    /// `tipo_documento`: affiliate document type.
    pub doc_type: String,
    /// `numero_documento`: affiliate document number.
    pub doc_number: String,
    /// `fecha`: dispensing date (`yyyy-MM-dd[ HH:mm[:ss]]`, `dd/MM/yyyy`, ...).
    pub date: String,
    /// `articulo`: article name (cut to 30 UTF-16 units).
    pub article: String,
    /// `cantidad`: quantity (Java `Integer.parseInt`).
    pub quantity: String,
    /// `valor_unitario`: unit value (Java money rule).
    pub unit_value: String,
    /// `valor_total`: total value.
    pub total_value: String,
    /// `cuota`: copay.
    pub copay: String,
    /// `numero_autorizacion`: authorization number (other services only).
    pub authorization: String,
    /// `mipres`: gates `idMIPRES` (non-empty means the id is sent).
    pub mipres: String,
    /// `numero_formula`: the value sent as `idMIPRES` (Java parity, see docs/rips.md).
    pub formula_number: String,
    /// `codigo_diagnostico`.
    pub diagnosis_code: String,
    /// `id_medico`: prescriber id (default `11111111`).
    pub prescriber_id: String,
    /// `cum`: billing CUM after the enrichment (`codTecnologiaSalud`).
    pub cum: String,
    /// `tipo_producto`: `714` medicine, `799` other service.
    pub product_type: String,
    /// `forma_farmaceutica`.
    pub pharmaceutical_form: String,
    /// `concentracion` (`UPR` leaves the line out).
    pub concentration: String,
    /// `unidad_medida`.
    pub measure_unit: String,
}

/// A prepared invoice with its lines in convenio order (Go `convenio.Invoice`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RipsInput {
    /// Normalized invoice number (`domain::normalize_invoice`).
    pub invoice: String,
    /// The invoice's lines in convenio order.
    pub lines: Vec<RipsLine>,
    /// The MIPRES delivery ids (`Invoice.id_mipres`) in the order the NO PBS
    /// lines (non-empty `mipres` column) were checked by `convenio preparar`.
    /// The builder pairs them with those lines in order; a count that does not
    /// match the number of NO PBS lines is a data error.
    pub id_mipres: Vec<String>,
}
