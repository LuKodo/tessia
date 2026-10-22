//! The convenio invoice shape `conciliar` reads and rewrites.
//!
//! These structs mirror Go's `convenio.Line`, `convenio.LineAudit` and
//! `convenio.InvoiceAudit` (same fields, same JSON names) and are the input and
//! output of the pure rules in [`super::rules`]. The dataset store adapts its
//! own rows to them (`Line` carries every field of a convenio line so that a
//! rebuild keeps the patient, authorization and diagnosis data of the line it
//! copies).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Origin of a line updated from the XML (`LineAudit::origin`).
pub const ORIGIN_MATCHED: &str = "CSV_CONCILIADO_XML";
/// Origin of a line the convenio did not have.
pub const ORIGIN_ADDED: &str = "LINEA_AGREGADA_DESDE_XML";

fn is_empty(s: &String) -> bool {
    s.is_empty()
}

/// Marks a line rebuilt from the FEV XML: quantity and values come from the
/// XML, the date from Medicar.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LineAudit {
    #[serde(rename = "origen")]
    pub origin: String,
    #[serde(rename = "identificador_linea_xml")]
    pub xml_identifier: String,
    /// Always `MEDICAR`.
    #[serde(rename = "fuente_fecha")]
    pub date_source: String,
    /// Always `XML`.
    #[serde(rename = "fuente_cantidad_valor")]
    pub value_source: String,
}

/// One convenio line (Go `convenio.Line`). Amounts and quantities are the text
/// the dataset keeps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Line {
    pub source_line: i64,
    #[serde(skip_serializing_if = "is_empty")]
    pub nit: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub plan: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub subplan: String,
    /// Normalized invoice number.
    #[serde(rename = "factura")]
    pub invoice: String,
    /// Invoice number as exported.
    #[serde(rename = "factura_raw")]
    pub invoice_raw: String,
    #[serde(rename = "tipo_documento")]
    pub doc_type: String,
    #[serde(rename = "numero_documento")]
    pub doc_number: String,
    #[serde(rename = "beneficiario", skip_serializing_if = "is_empty")]
    pub beneficiary: String,
    #[serde(rename = "punto_venta", skip_serializing_if = "is_empty")]
    pub point_of_sale: String,
    #[serde(rename = "numero_formula", skip_serializing_if = "is_empty")]
    pub formula_number: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub ssc: String,
    #[serde(rename = "tipo_formula", skip_serializing_if = "is_empty")]
    pub formula_type: String,
    #[serde(rename = "fecha")]
    pub date: String,
    #[serde(rename = "lote", skip_serializing_if = "is_empty")]
    pub lot: String,
    pub plu: String,
    #[serde(rename = "articulo")]
    pub article: String,
    #[serde(rename = "cantidad")]
    pub quantity: String,
    #[serde(rename = "valor_unitario")]
    pub unit_value: String,
    #[serde(rename = "costo_unitario", skip_serializing_if = "is_empty")]
    pub unit_cost: String,
    #[serde(rename = "valor_total")]
    pub total_value: String,
    #[serde(rename = "cuota")]
    pub copay: String,
    #[serde(rename = "numero_autorizacion", skip_serializing_if = "is_empty")]
    pub authorization: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub mipres: String,
    /// CUM as exported.
    #[serde(rename = "cum_convenio")]
    pub agreement_cum: String,
    #[serde(rename = "codigo_diagnostico", skip_serializing_if = "is_empty")]
    pub diagnosis_code: String,
    #[serde(rename = "nombre_diagnostico", skip_serializing_if = "is_empty")]
    pub diagnosis_name: String,
    #[serde(rename = "causal_exento", skip_serializing_if = "is_empty")]
    pub causal_exento: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub ips: String,
    #[serde(rename = "id_medico", skip_serializing_if = "is_empty")]
    pub prescriber_id: String,
    #[serde(rename = "nombre_medico", skip_serializing_if = "is_empty")]
    pub prescriber_name: String,
    #[serde(rename = "genero", skip_serializing_if = "is_empty")]
    pub gender: String,
    #[serde(rename = "fecha_nacimiento", skip_serializing_if = "is_empty")]
    pub birth_date: String,
    #[serde(rename = "departamento", skip_serializing_if = "is_empty")]
    pub department: String,
    #[serde(rename = "municipio", skip_serializing_if = "is_empty")]
    pub municipality: String,
    #[serde(rename = "clasificacion", skip_serializing_if = "is_empty")]
    pub classification: String,
    /// Billing CUM (nota técnica).
    pub cum: String,
    #[serde(rename = "tipo_producto", skip_serializing_if = "is_empty")]
    pub product_type: String,
    #[serde(rename = "forma_farmaceutica", skip_serializing_if = "is_empty")]
    pub pharmaceutical_form: String,
    #[serde(rename = "concentracion", skip_serializing_if = "is_empty")]
    pub concentration: String,
    #[serde(rename = "unidad_medida", skip_serializing_if = "is_empty")]
    pub measure_unit: String,
    #[serde(rename = "precio_pactado", skip_serializing_if = "is_empty")]
    pub agreed_price: String,
    /// Set on lines rebuilt by `conciliar`.
    #[serde(rename = "conciliacion", skip_serializing_if = "Option::is_none")]
    pub conciliacion: Option<LineAudit>,
}

/// Evidence of a reconciliation (Go `convenio.InvoiceAudit`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceAudit {
    #[serde(rename = "estado")]
    pub state: String,
    #[serde(rename = "fecha_formula")]
    pub formula_date: String,
    pub ssc: String,
    #[serde(rename = "valor_xml")]
    pub xml_total: String,
    #[serde(rename = "valor_convenio_anterior")]
    pub previous_total: String,
    #[serde(rename = "valor_convenio_conciliado")]
    pub reconciled_total: String,
    #[serde(rename = "conciliado_en")]
    pub reconciled_at: DateTime<Utc>,
    /// Lines before the FIRST reconciliation.
    #[serde(rename = "lineas_originales")]
    pub original_lines: Vec<Line>,
    /// Indexes of the original lines no XML line confirmed.
    #[serde(
        rename = "no_confirmadas_xml",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub unconfirmed: Vec<usize>,
}

/// An invoice of the dataset: its lines plus the reconciliation audit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Invoice {
    /// Normalized invoice number.
    pub invoice: String,
    pub lines: Vec<Line>,
    /// MIPRES delivery ids (they survive a reconciliation).
    pub id_mipres: String,
    pub conciliacion: Option<InvoiceAudit>,
}
