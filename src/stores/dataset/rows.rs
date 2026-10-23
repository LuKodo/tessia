//! Row types and the encode/decode between [`crate::domain::dataset`] and the
//! `dataset_*` tables.

use chrono::{DateTime, NaiveDate, Utc};
use diesel::prelude::*;
use rust_decimal::Decimal;
use serde_json::Value;

use super::error::{DatasetError, DatasetResult};
use super::schema::{dataset_factura, dataset_linea, dataset_preparacion};
use super::summary::InvoiceSummary;
use crate::domain::dataset::{Invoice, InvoiceState, Line, Preparation, Stats};
use crate::domain::{LoadInfo, LoadOrigin, normalize_invoice};

/// `dataset_preparacion` as read.
#[derive(Queryable, Selectable)]
#[diesel(table_name = dataset_preparacion)]
pub(super) struct PrepRow {
    pub load_id: String,
    pub load_name: String,
    pub load_from: Option<NaiveDate>,
    pub load_to: Option<NaiveDate>,
    pub load_origin: String,
    pub source: String,
    pub max_amount: Option<Decimal>,
    pub automatic_subplans: Value,
    pub nt_version: String,
    pub prepared_format: bool,
    pub stats: Value,
    pub skipped_lines: Value,
    pub prepared_at: DateTime<Utc>,
}

impl PrepRow {
    pub fn into_preparation(self) -> DatasetResult<Preparation> {
        let decode = |what: &str, e: serde_json::Error| {
            DatasetError::Invalid(format!("{what} guardado ilegible: {e}"))
        };
        Ok(Preparation {
            load: LoadInfo {
                name: self.load_name,
                id: self.load_id,
                from: self.load_from,
                to: self.load_to,
                origin: LoadOrigin::parse(&self.load_origin).unwrap_or(LoadOrigin::Unknown),
            },
            source: self.source,
            max_amount: self.max_amount,
            automatic_subplans: serde_json::from_value(self.automatic_subplans)
                .map_err(|e| decode("automatic_subplans", e))?,
            nt_version: self.nt_version,
            prepared_format: self.prepared_format,
            prepared_at: Some(self.prepared_at),
            stats: serde_json::from_value::<Stats>(self.stats).map_err(|e| decode("stats", e))?,
            skipped_lines: serde_json::from_value(self.skipped_lines)
                .map_err(|e| decode("skipped_lines", e))?,
        })
    }
}

/// `dataset_preparacion` as written (insert or update of the whole header).
#[derive(Insertable, AsChangeset)]
#[diesel(table_name = dataset_preparacion, treat_none_as_null = true)]
pub(super) struct NewPrep {
    pub cargue_seq: i64,
    pub load_id: String,
    pub load_name: String,
    pub load_from: Option<NaiveDate>,
    pub load_to: Option<NaiveDate>,
    pub load_origin: String,
    pub source: String,
    pub max_amount: Option<Decimal>,
    pub automatic_subplans: Value,
    pub nt_version: String,
    pub prepared_format: bool,
    pub stats: Value,
    pub skipped_lines: Value,
    pub prepared_at: DateTime<Utc>,
}

impl NewPrep {
    pub fn new(
        cargue: i64,
        prep: &Preparation,
        prepared_at: DateTime<Utc>,
        stats: Value,
        skipped_lines: Value,
        prepared_format: bool,
    ) -> DatasetResult<Self> {
        Ok(Self {
            cargue_seq: cargue,
            load_id: prep.load.id.clone(),
            load_name: prep.load.name.clone(),
            load_from: prep.load.from,
            load_to: prep.load.to,
            load_origin: prep.load.origin.as_str().to_owned(),
            source: prep.source.clone(),
            max_amount: prep.max_amount,
            automatic_subplans: to_json(&prep.automatic_subplans)?,
            nt_version: prep.nt_version.clone(),
            prepared_format,
            stats,
            skipped_lines,
            prepared_at,
        })
    }
}

pub(super) fn to_json<T: serde::Serialize>(value: &T) -> DatasetResult<Value> {
    serde_json::to_value(value)
        .map_err(|e| DatasetError::Invalid(format!("no se pudo codificar JSON: {e}")))
}

/// `dataset_factura` as written.
#[derive(Insertable)]
#[diesel(table_name = dataset_factura)]
pub(super) struct NewFactura {
    pub cargue_seq: i64,
    pub factura: String,
    pub factura_norm: String,
    pub estado: &'static str,
    pub codigo: String,
    pub motivo: String,
    pub id_mipres: String,
    pub orden: i32,
    pub conciliacion: Option<Value>,
    pub version: i64,
    pub fecha_factura_txt: String,
    pub ssc: String,
    pub subplan: String,
    pub autorizacion: String,
    pub valor_convenio: Option<Decimal>,
}

/// `dataset_linea` as written.
#[derive(Insertable)]
#[diesel(table_name = dataset_linea)]
pub(super) struct NewLinea {
    pub cargue_seq: i64,
    pub factura: String,
    pub orden: i32,
    pub source_line: i32,
    pub total_value: String,
    pub data: Value,
}

/// One invoice of a dataset before it gets its position and version.
pub(super) struct FacturaDraft {
    pub name: String,
    pub state: InvoiceState,
    pub code: String,
    pub reason: String,
    pub mipres: String,
    pub audit: Option<Value>,
    /// Stored summary columns (empty for excluded and manual-review invoices).
    pub summary: InvoiceSummary,
}

impl FacturaDraft {
    pub fn into_row(self, cargue: i64, orden: i32, version: i64) -> NewFactura {
        NewFactura {
            cargue_seq: cargue,
            factura_norm: normalize_invoice(&self.name),
            factura: self.name,
            estado: self.state.as_str(),
            codigo: self.code,
            motivo: self.reason,
            id_mipres: self.mipres,
            orden,
            conciliacion: self.audit,
            version,
            fecha_factura_txt: self.summary.date,
            ssc: self.summary.ssc,
            subplan: self.summary.subplan,
            autorizacion: self.summary.authorization,
            valor_convenio: self.summary.agreement_value,
        }
    }
}

pub(super) fn encode_audit(invoice: &Invoice) -> DatasetResult<Option<Value>> {
    invoice.conciliacion.as_ref().map(to_json).transpose()
}

pub(super) fn line_rows(cargue: i64, invoice: &Invoice) -> DatasetResult<Vec<NewLinea>> {
    invoice
        .lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            Ok(NewLinea {
                cargue_seq: cargue,
                factura: invoice.invoice.clone(),
                orden: i32::try_from(i).map_err(|_| {
                    DatasetError::Invalid("demasiadas líneas en una factura".into())
                })?,
                source_line: line.source_line,
                total_value: line.total_value.clone(),
                data: to_json(line)?,
            })
        })
        .collect()
}

pub(super) fn decode_line(data: Value) -> DatasetResult<Line> {
    serde_json::from_value(data)
        .map_err(|e| DatasetError::Invalid(format!("línea guardada ilegible: {e}")))
}
