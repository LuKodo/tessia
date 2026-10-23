//! The Mutual Ser portal result of an invoice (Go `cmd_serve_portal.go` and
//! `internal/portalcargue`):
//!
//! * `GET /api/facturas/{f}/portal`: the stored result, fetched lazily the
//!   first time (and again when it is not final and older than 30 s);
//! * `POST /api/facturas/{f}/portal/actualizar`: forces a portal query.
//!
//! A portal failure is a 200 carrying the stored data and `error`; only a
//! store failure is a 500 with a generic message. Always `Cache-Control:
//! no-store`: the answer carries business data (the portal user, findings).

use std::sync::Arc;

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::api::{
    ApiState,
    errors::{ApiError, ApiResult},
    request::{Request, Response},
};
use crate::domain::valid_invoice;
use crate::phases::radicacion::portal::{
    LookupOptions, LookupResult, PortalApi, PortalStore, lookup, portal_client,
};

const READ_FAILED: &str = "no se pudo leer el resultado del portal";

/// The portal time zone (Colombia has no DST).
fn portal_zone() -> FixedOffset {
    FixedOffset::west_opt(5 * 3600).unwrap_or_else(|| unreachable!("constant offset"))
}

#[derive(Debug, Serialize)]
struct FindingView {
    codigo: String,
    tipo: String,
    campo: String,
    detalle: String,
}

/// The JSON contract of the portal endpoints; the dashboard UI is written
/// against these exact field names.
#[derive(Debug, Serialize)]
struct PortalView {
    factura: String,
    consultado: bool,
    #[serde(rename = "final")]
    is_final: bool,
    estado: String,
    remision: String,
    fecha_cargue: String,
    fecha_cargue_texto: String,
    usuario: String,
    servicios: String,
    modalidad: String,
    nombre_archivo: String,
    id_archivo: String,
    id_cargue: String,
    id_evento: String,
    tipo_evento: String,
    tipo_error: String,
    codigo_error: String,
    devuelto: bool,
    fecha_expedicion: String,
    receipt_name: String,
    documento_soporte: Value,
    advertencias: i32,
    errores: i32,
    hallazgos: Vec<FindingView>,
    consultas: i32,
    consultado_en: String,
    origen: String,
    mensaje: String,
    error: String,
}

/// The stored `documento_soporte` as a JSON object, or `null` when it is
/// empty, not valid JSON or not an object.
fn support_document(stored: &str) -> Value {
    let text = stored.trim();
    if !text.starts_with('{') {
        return Value::Null;
    }
    serde_json::from_str::<Value>(text)
        .ok()
        .filter(Value::is_object)
        .unwrap_or(Value::Null)
}

fn view(invoice: &str, result: LookupResult) -> PortalView {
    let LookupResult {
        snapshot,
        source,
        message,
        error,
    } = result;
    let mut view = PortalView {
        factura: invoice.to_owned(),
        consultado: false,
        is_final: false,
        estado: String::new(),
        remision: String::new(),
        fecha_cargue: String::new(),
        fecha_cargue_texto: String::new(),
        usuario: String::new(),
        servicios: String::new(),
        modalidad: String::new(),
        nombre_archivo: String::new(),
        id_archivo: String::new(),
        id_cargue: String::new(),
        id_evento: String::new(),
        tipo_evento: String::new(),
        tipo_error: String::new(),
        codigo_error: String::new(),
        devuelto: false,
        fecha_expedicion: String::new(),
        receipt_name: String::new(),
        documento_soporte: Value::Null,
        advertencias: 0,
        errores: 0,
        hallazgos: Vec::new(),
        consultas: 0,
        consultado_en: String::new(),
        origen: source.to_owned(),
        mensaje: message,
        error,
    };
    if !snapshot.found {
        return view;
    }
    let record = snapshot.record;
    view.consultado = true;
    view.is_final = snapshot.is_final;
    view.estado = record.status;
    view.remision = record.remission;
    if let Some(at) = record.uploaded_at {
        view.fecha_cargue = at
            .with_timezone(&portal_zone())
            .to_rfc3339_opts(SecondsFormat::Secs, false);
    }
    view.fecha_cargue_texto = record.uploaded_at_text;
    view.usuario = record.user;
    view.servicios = record.services;
    view.modalidad = record.modality;
    view.nombre_archivo = record.file_name;
    view.id_archivo = record.file_id;
    view.id_cargue = record.upload_id;
    view.id_evento = record.event_id;
    view.tipo_evento = record.event_type;
    view.tipo_error = record.error_type;
    view.codigo_error = record.error_code;
    view.devuelto = record.returned;
    view.fecha_expedicion = record.issue_date;
    view.receipt_name = record.receipt_name;
    view.documento_soporte = support_document(&record.support_document);
    view.advertencias = snapshot.warnings;
    view.errores = snapshot.errors;
    view.hallazgos = snapshot
        .findings
        .into_iter()
        .map(|f| FindingView {
            codigo: f.code,
            tipo: f.kind,
            campo: f.field,
            detalle: f.detail,
        })
        .collect();
    view.consultas = snapshot.queries;
    if let Some(at) = snapshot.queried_at {
        view.consultado_en = utc(at);
    }
    view
}

fn utc(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Trimmed, upper-cased and without dashes, like the rest of the code base.
fn invoice_key(raw: &str) -> String {
    raw.trim().replace('-', "").to_uppercase()
}

fn handle(state: &ApiState, req: &Request, force: bool) -> ApiResult {
    let invoice = invoice_key(req.param(0));
    if !valid_invoice(&invoice) {
        return Err(ApiError::bad_request("factura inválida"));
    }
    let store = PortalStore::new(state.pool.clone());
    let client = || -> Option<Arc<dyn PortalApi>> {
        match portal_client(&state.ctx) {
            Ok(client) => Some(client),
            Err(err) => {
                tracing::info!(error = %err, "portal client unavailable");
                None
            }
        }
    };
    let result = lookup(
        &store,
        client,
        &invoice,
        force,
        Utc::now(),
        &LookupOptions::default(),
    )
    .map_err(|err| {
        tracing::warn!(invoice = %invoice, error = %err, "portal result lookup failed");
        ApiError::internal(READ_FAILED)
    })?;
    Ok(Response::json(&view(&invoice, result)).with_header("Cache-Control", "no-store"))
}

pub fn get(state: &ApiState, req: &Request) -> ApiResult {
    handle(state, req, false)
}

pub fn refresh(state: &ApiState, req: &Request) -> ApiResult {
    handle(state, req, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_support_document_is_kept_only_when_it_is_a_json_object() {
        assert_eq!(support_document(""), Value::Null);
        assert_eq!(support_document("  "), Value::Null);
        assert_eq!(support_document("[1]"), Value::Null);
        assert_eq!(support_document("{oops"), Value::Null);
        assert_eq!(support_document("\"x\""), Value::Null);
        assert_eq!(
            support_document(r#" {"id":"s1","estado":3} "#),
            serde_json::json!({"id": "s1", "estado": 3})
        );
    }

    #[test]
    fn invoices_are_trimmed_stripped_of_dashes_and_upper_cased() {
        assert_eq!(invoice_key(" fev-12 "), "FEV12");
        assert!(valid_invoice(&invoice_key("fev-12")));
        assert!(!valid_invoice(&invoice_key("12-fev")));
        assert!(!valid_invoice(&invoice_key("")));
    }

    #[test]
    fn nothing_found_gives_the_empty_view_with_notes() {
        let result = LookupResult {
            message: "El portal aún no tiene registro de esta factura".into(),
            source: "portal",
            ..LookupResult::default()
        };
        let value = serde_json::to_value(view("FEV1", result)).expect("json");
        assert_eq!(value["consultado"], false);
        assert_eq!(value["hallazgos"], serde_json::json!([]));
        assert_eq!(value["documento_soporte"], Value::Null);
        assert_eq!(value["origen"], "portal");
        assert_eq!(value["error"], "");
    }
}
