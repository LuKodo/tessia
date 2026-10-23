//! `GET /api/facturas/{f}/seguimiento` and the tracking row every list route
//! shares (Go `dashboard.TrackingRow`, `InvoiceTracking`, `TrackingInvoice`).

use serde::Serialize;

use crate::api::read::derive::{
    self, Derived, Dot, apply_conciliar_block, mipres_column, plain_amount, progress_detail,
    quoted, rfc3339, short_detail, valid_cargue_id,
};
use crate::api::{
    ApiState,
    errors::{ApiError, ApiResult},
    request::{Request, Response},
};
use crate::domain::dataset::{InvoiceState, Line};
use crate::domain::{normalize_invoice, valid_invoice};
use crate::stores::dataset::DatasetStore;
use crate::stores::dataset::dashboard::{DatasetRow, InvoiceHit, NativeScope, PageExtras};

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

/// One invoice of the "Seguimiento de RIPS" table. Field names, order and
/// omission rules are `ui/src/types/api.ts` `TrackingRow` (Go `omitempty`).
#[derive(Debug, Clone, Serialize)]
pub struct TrackingRow {
    pub factura: String,
    pub resultado: &'static str,
    pub cargue: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cargue_nombre: String,
    /// ZIP XML/PDF, RIPS JSON, TXT CUV and ZIP CRC, in that order.
    pub artefactos: [Dot; 4],
    /// Mutual Ser filing steps 1-4.
    pub radicacion: [Dot; 4],
    #[serde(skip_serializing_if = "String::is_empty")]
    pub estado: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub estado_detalle: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fecha_factura: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ssc: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub subplan: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub valor_xml: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub valor_convenio: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub autorizacion: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id_mipres: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actualizado_en: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub requiere_nota_ajuste: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub requiere_nota_credito: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub vencida_rffe022: bool,
    pub pipeline: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alertas: Vec<&'static str>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fecha_inicio_cargue: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fecha_fin_cargue: String,
    pub portal_consultado: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub portal_estado: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub portal_advertencias: i32,
    #[serde(skip_serializing_if = "is_zero")]
    pub portal_errores: i32,
    #[serde(skip_serializing_if = "is_false")]
    pub portal_final: bool,
}

/// One convenio line of the invoice detail view.
#[derive(Debug, Clone, Serialize)]
pub struct AgreementLine {
    pub ssc: String,
    pub cum: String,
    pub valor_unitario: String,
    pub valor_total: String,
    pub autorizacion: String,
    pub id_mipres: String,
}

/// The full tracking row of one invoice: the list row with its whole estado
/// detail plus its convenio lines.
#[derive(Debug, Serialize)]
pub struct InvoiceTracking {
    #[serde(flatten)]
    pub row: TrackingRow,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lineas_convenio: Vec<AgreementLine>,
}

/// Builds the row of one invoice from its dataset row, what was derived from
/// the native state and the side data of its page. `estado_detalle` is the
/// whole text; list routes cut it with [`short_detail`].
pub fn build_row(row: &DatasetRow, d: &Derived, extras: &PageExtras) -> TrackingRow {
    let detail_rows = extras.details.get(&row.factura_norm);
    let mut r = TrackingRow {
        factura: row.factura.clone(),
        resultado: derive::resultado(d.state),
        cargue: row.load_id.clone(),
        cargue_nombre: row.load_name.clone(),
        artefactos: d.artifacts,
        radicacion: d.filing,
        estado: String::new(),
        estado_detalle: String::new(),
        fecha_factura: String::new(),
        ssc: String::new(),
        subplan: String::new(),
        valor_xml: String::new(),
        valor_convenio: String::new(),
        autorizacion: String::new(),
        id_mipres: String::new(),
        actualizado_en: None,
        requiere_nota_ajuste: d.note_adjustment,
        requiere_nota_credito: d.note_credit,
        vencida_rffe022: d.expired,
        pipeline: d.pipeline,
        alertas: d.alerts.clone(),
        fecha_inicio_cargue: row.load_from.map(|v| v.to_string()).unwrap_or_default(),
        fecha_fin_cargue: row.load_to.map(|v| v.to_string()).unwrap_or_default(),
        portal_consultado: false,
        portal_estado: String::new(),
        portal_advertencias: 0,
        portal_errores: 0,
        portal_final: false,
    };
    match d.state {
        InvoiceState::Prepared => fill_prepared(&mut r, row, d, extras),
        InvoiceState::Excluded => r.estado_detalle = row.motivo.clone(),
        InvoiceState::ManualReview => r.estado_detalle = format!("{}: {}", row.codigo, row.motivo),
    }
    if let Some(portal) = extras.portal.get(&row.factura_norm) {
        r.portal_consultado = true;
        r.portal_estado = portal.estado.clone();
        r.portal_advertencias = portal.advertencias;
        r.portal_errores = portal.errores;
        r.portal_final = portal.es_final;
    }
    apply_conciliar_block(&mut r.estado_detalle, detail_rows);
    r
}

fn fill_prepared(r: &mut TrackingRow, row: &DatasetRow, d: &Derived, extras: &PageExtras) {
    r.fecha_factura = row.fecha.clone();
    r.ssc = row.ssc.clone();
    r.subplan = row.subplan.clone();
    r.autorizacion = row.autorizacion.clone();
    r.valor_convenio = row.valor.map(plain_amount).unwrap_or_default();
    r.id_mipres = mipres_column(&row.subplan, &row.id_mipres);
    if let Some(xml) = extras.xml.get(&row.factura_norm) {
        r.valor_xml = plain_amount(*xml);
    }
    if let Some(estado) = d.estado {
        r.estado = estado.to_owned();
        r.actualizado_en = d.updated_at.map(rfc3339);
        if let Some(detail) = extras.details.get(&row.factura_norm) {
            let text = progress_detail(detail);
            if !text.is_empty() {
                r.estado_detalle = text;
            }
        }
    }
}

/// The list form of a row: the estado detail cut to 90 characters.
pub fn into_list_row(mut row: TrackingRow) -> TrackingRow {
    row.estado_detalle = short_detail(&row.estado_detalle);
    row
}

/// The detail view's convenio lines: the CUM is the billing one (the
/// agreement CUM until enriched) and the unit value the agreed price (the
/// exported unit value when the nota técnica gave none).
fn agreement_lines(lines: &[Line]) -> Vec<AgreementLine> {
    lines
        .iter()
        .map(|l| AgreementLine {
            ssc: l.ssc.clone(),
            cum: first_non_empty(&l.cum, &l.agreement_cum),
            valor_unitario: first_non_empty(&l.agreed_price, &l.unit_value),
            valor_total: l.total_value.clone(),
            autorizacion: l.authorization.clone(),
            id_mipres: l.mipres.clone(),
        })
        .collect()
}

fn first_non_empty(a: &str, b: &str) -> String {
    if a.is_empty() { b } else { a }.to_owned()
}

fn failure(what: &str, err: &dyn std::fmt::Display) -> ApiError {
    tracing::error!(error = %err, "dashboard: {what}");
    ApiError::internal("no se pudo leer el seguimiento de la factura")
}

/// `GET /api/facturas/{f}/seguimiento?cargue=`.
pub fn get(state: &ApiState, req: &Request) -> ApiResult {
    let invoice = normalize_invoice(req.param(0));
    if !valid_invoice(&invoice) {
        return Err(ApiError::bad_request("factura inválida"));
    }
    let cargue = req.query_one("cargue").unwrap_or("");
    if !cargue.is_empty() && !valid_cargue_id(cargue) {
        return Err(ApiError::bad_request("cargue inválido"));
    }
    let store = DatasetStore::new(state.pool.clone());
    let hit = store
        .dashboard_invoice(&invoice, (!cargue.is_empty()).then_some(cargue))
        .map_err(|e| failure("invoice lookup", &e))?;
    let row = match hit {
        InvoiceHit::NoCargue => {
            return Err(ApiError::not_found(format!(
                "dashboard: cargue not found: {}",
                quoted(cargue)
            )));
        }
        InvoiceHit::NotFound => {
            return Err(ApiError::not_found(format!(
                "dashboard: invoice not found: {}",
                quoted(&invoice)
            )));
        }
        InvoiceHit::Found(row) => *row,
    };
    let names = [row.factura_norm.clone()];
    let native = store
        .dashboard_native(NativeScope::Names(&names))
        .map_err(|e| failure("native state", &e))?;
    let d = derive::derive(
        derive::parse_state(&row.estado),
        &row.autorizacion,
        native.get(&row.factura_norm),
    );
    let extras = store
        .dashboard_extras(&names)
        .map_err(|e| failure("page extras", &e))?;
    let lineas_convenio = if d.state == InvoiceState::Prepared {
        let ds = store
            .read(row.load_seq, Some(&names))
            .map_err(|e| failure("invoice lines", &e))?;
        ds.invoices
            .first()
            .map(|inv| agreement_lines(&inv.lines))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(Response::json(&InvoiceTracking {
        row: build_row(&row, &d, &extras),
        lineas_convenio,
    }))
}
