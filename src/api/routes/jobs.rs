//! The job routes (Go `cmd_serve.go`, `cmd_serve_download.go`,
//! `cmd_serve_retry.go`): start a `proceso run`, a `convenio descargar` or the
//! `conciliar` retry of one invoice, and read, cancel and list jobs.
//!
//! Validation follows Go's order. Request keys are matched case-insensitively
//! like Go's JSON decoder (the UI sends `Cargue`, `From`, `To`, `Limit`,
//! `Desde`, `Hasta`). A step name that `--from`/`--to` would reject is answered
//! with 400 up front (Go let the job fail with the same message).

use chrono::Local;
use serde_json::{Map, Value};

use crate::api::{
    ApiState,
    errors::{ApiError, ApiResult},
    request::{Request, Response},
};
use crate::context::AppContext;
use crate::db::DbError;
use crate::domain::{Phase, PhaseState, normalize_invoice};
use crate::jobs::{NoJobRunning, StartError, StartOptions};
use crate::phases::common::PhaseRequest;
use crate::phases::convenio_descargar::{file_name, resolve_range};
use crate::phases::proceso::steps::{self, Active, ORDER};
use crate::phases::proceso::{Events, Mode, RunConfig};
use crate::stores::cargues::CarguesStore;
use crate::stores::dataset::DatasetStore;
use crate::stores::proceso::{ProcesoState, ProcesoStore};

/// Cap of the JSON bodies of the job-starting routes.
const MAX_JSON_BODY: usize = 64 << 10;
/// Cap of the retry body.
const MAX_RETRY_BODY: usize = 4096;
const BAD_RETRY_BODY: &str = "cuerpo JSON inválido";
const UNVERIFIABLE: &str = "no se pudo verificar el estado persistido de la factura";
const CARGUE_NOT_FOUND: &str = "dashboard: cargue not found";

impl From<StartError> for ApiError {
    fn from(err: StartError) -> Self {
        let message = err.to_string();
        match err {
            StartError::Running { .. } => ApiError::conflict(message),
            StartError::Stopping => ApiError::unavailable(message),
            StartError::Spawn(_) => ApiError::internal(message),
        }
    }
}

impl From<NoJobRunning> for ApiError {
    fn from(err: NoJobRunning) -> Self {
        ApiError::conflict(err.to_string())
    }
}

// ------------------------------------------------------------------- run

/// `POST /api/run`: `proceso run` over an existing cargue.
pub fn run(state: &ApiState, req: &Request) -> ApiResult {
    let body = lenient_body(req)?;
    let cargue = string_field(&body, "Cargue")?;
    let from = string_field(&body, "From")?;
    let to = string_field(&body, "To")?;
    let limit = int_field(&body, "Limit")?;

    if !valid_cargue_id(&cargue) {
        return Err(ApiError::bad_request(format!("cargue invalido {cargue:?}")));
    }
    if limit < 0 {
        return Err(ApiError::bad_request("--limit must be >= 0"));
    }
    let exists = CarguesStore::new(state.pool.clone())
        .newest(&cargue)
        .map_err(|err| internal("no se pudo consultar el cargue", &err))?;
    if exists.is_none() {
        return Err(not_found_cargue(&cargue));
    }
    let active = steps::select(non_empty(&from), non_empty(&to), &[])
        .map_err(|err| ApiError::bad_request(err.to_string()))?;

    let mut cfg = RunConfig::new(&cargue);
    cfg.limit =
        usize::try_from(limit).map_err(|_| ApiError::bad_request("--limit is too large"))?;
    cfg.mode = Mode::Text;
    cfg.active = active.clone();
    let options = StartOptions {
        cargue: Some(cargue),
        plan: active.labels().iter().map(|l| (*l).to_owned()).collect(),
        phases: phase_cards(&active),
    };
    let backend = state.jobs.backend().clone();
    let work =
        Box::new(move |ctx: &AppContext, events: &Events| (backend.proceso)(ctx, &cfg, events));
    start(state, "proceso run", options, work)
}

/// The phase cards a run fills, in pipeline order. Convenio preparar (step 0)
/// has its own strip and never emits phase events.
fn phase_cards(active: &Active) -> Vec<(String, String)> {
    ORDER
        .iter()
        .filter(|name| active.has(name) && **name != "convenio")
        .map(|name| ((*name).to_owned(), steps::label(name).to_owned()))
        .collect()
}

// -------------------------------------------------------------- download

/// `POST /api/download`: `convenio descargar` for a date range.
pub fn download(state: &ApiState, req: &Request) -> ApiResult {
    let body = lenient_body(req)?;
    let desde = string_field(&body, "Desde")?;
    let hasta = string_field(&body, "Hasta")?;
    let (from, to) = resolve_range(Some(&desde), Some(&hasta), false, Local::now().date_naive())
        .map_err(|err| ApiError::bad_request(err.to_string()))?;

    let cargue = file_name(from, to);
    let cargue = cargue.trim_end_matches(".csv");
    let exists = CarguesStore::new(state.pool.clone())
        .newest(cargue)
        .map_err(|err| internal("no se pudo consultar el cargue", &err))?;
    if exists.is_some() {
        return Err(ApiError::conflict(
            "Ya existe un cargue para esas fechas; no se reemplazó.",
        ));
    }
    let backend = state.jobs.backend().clone();
    let work =
        Box::new(move |ctx: &AppContext, _events: &Events| (backend.download)(ctx, from, to));
    start(state, "convenio descargar", StartOptions::default(), work)
}

// ------------------------------------------------------------- reintentar

/// `POST /api/cargues/{id}/reintentar`: `conciliar` for one invoice whose
/// `fev_valor` is waiting for a manual review of a VALOR_DIFERENTE.
pub fn reintentar(state: &ApiState, req: &Request) -> ApiResult {
    let invoice = retry_invoice(&req.body)?;
    if invoice.trim().is_empty() {
        return Err(ApiError::bad_request("factura requerida"));
    }
    let cargue = req.param(0);
    if !valid_cargue_id(cargue) {
        return Err(ApiError::not_found(CARGUE_NOT_FOUND));
    }
    let invoice = normalize_invoice(&invoice);
    let verdict = check_retry(state, cargue, &invoice).map_err(|err| {
        tracing::warn!(error = %err, "retry eligibility could not be verified");
        ApiError::unavailable(UNVERIFIABLE)
    })?;
    match verdict {
        Retry::UnknownCargue => return Err(not_found_cargue(cargue)),
        Retry::NotEligible => {
            return Err(ApiError::conflict(
                "la factura no tiene una revisión manual VALOR_DIFERENTE elegible",
            ));
        }
        Retry::Eligible => {}
    }
    let request = PhaseRequest {
        cargue_id: cargue.to_owned(),
        invoices: Some(vec![invoice.clone()]),
        workers: None,
    };
    let backend = state.jobs.backend().clone();
    let work =
        Box::new(move |ctx: &AppContext, _events: &Events| (backend.conciliar)(ctx, &request));
    let kind = format!("conciliar invoice {invoice}");
    start(state, &kind, StartOptions::default(), work)
}

#[derive(Debug, PartialEq, Eq)]
enum Retry {
    UnknownCargue,
    NotEligible,
    Eligible,
}

/// Reads what the manual-review report reads for this invoice: the cargue, the
/// invoice's membership in its dataset and its `fev_valor` / `conciliacion`
/// states. The verdict itself is [`eligible`].
fn check_retry(state: &ApiState, cargue: &str, invoice: &str) -> Result<Retry, DbError> {
    let pool = state.pool.clone();
    if CarguesStore::new(pool.clone()).newest(cargue)?.is_none() {
        return Ok(Retry::UnknownCargue);
    }
    let datasets = DatasetStore::new(pool.clone());
    let Some(seq) = datasets.current_cargue(cargue).map_err(dataset_error)? else {
        return Ok(Retry::NotEligible);
    };
    let names = datasets.invoice_names(seq).map_err(dataset_error)?;
    if !names.iter().any(|name| name == invoice) {
        return Ok(Retry::NotEligible);
    }
    let one = [invoice.to_owned()];
    let proceso = ProcesoStore::new(pool);
    let fev = proceso.load(Phase::FevValor, Some(&one))?;
    let conciliar = proceso.load(Phase::Conciliacion, Some(&one))?;
    Ok(match fev.get(invoice) {
        Some(fev) if eligible(fev, conciliar.get(invoice)) => Retry::Eligible,
        _ => Retry::NotEligible,
    })
}

fn dataset_error(err: crate::stores::dataset::DatasetError) -> DbError {
    DbError::Other(err.to_string())
}

/// A retry is eligible when `fev_valor` is a manual review whose reason is
/// VALOR_DIFERENTE and `conciliar` has not ruled the invoice out for good
/// (BLOQUEADA_*) at or after that check.
fn eligible(fev: &ProcesoState, conciliar: Option<&ProcesoState>) -> bool {
    fev.state == PhaseState::RevisionManual
        && value_difference(&fev.motivo)
        && !conciliar.is_some_and(|c| permanent_block(c) && c.updated_at >= fev.updated_at)
}

/// The leading `CODE: ` of a reason (the report's `codigo`, regex
/// `^([A-Za-z][A-Za-z0-9_]*):\s+(.+)$`), empty when there is none.
fn reason_code(reason: &str) -> String {
    let text = reason.trim();
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    let code = &text[..end];
    if !code.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return String::new();
    }
    let Some(after) = text[end..].strip_prefix(':') else {
        return String::new();
    };
    let body = after.trim_start();
    if body.len() == after.len() || body.is_empty() {
        return String::new();
    }
    code.to_owned()
}

fn value_difference(reason: &str) -> bool {
    reason_code(reason) == "VALOR_DIFERENTE"
        || reason.split_whitespace().next() == Some("VALOR_DIFERENTE")
}

fn permanent_block(conciliar: &ProcesoState) -> bool {
    conciliar.state == PhaseState::Excluded
        && matches!(
            reason_code(&conciliar.motivo).as_str(),
            "BLOQUEADA_TARIFA" | "BLOQUEADA_MONTO"
        )
}

/// The `factura` of a retry body: at most 4 KiB, a single JSON value, only the
/// `factura` key (any case) and a string (or null) as its value.
fn retry_invoice(body: &[u8]) -> Result<String, ApiError> {
    let bad = || ApiError::bad_request(BAD_RETRY_BODY);
    if body.len() > MAX_RETRY_BODY {
        return Err(bad());
    }
    match serde_json::from_slice::<Value>(body).map_err(|_| bad())? {
        Value::Null => Ok(String::new()),
        Value::Object(object) => {
            let mut invoice = String::new();
            for (key, value) in object {
                if !key.eq_ignore_ascii_case("factura") {
                    return Err(bad());
                }
                match value {
                    Value::String(text) => invoice = text,
                    Value::Null => {}
                    _ => return Err(bad()),
                }
            }
            Ok(invoice)
        }
        _ => Err(bad()),
    }
}

// ------------------------------------------------------- current and co.

/// `GET /api/jobs/current`.
pub fn current(state: &ApiState, _req: &Request) -> ApiResult {
    Ok(Response::json(&state.jobs.current()))
}

/// `GET /api/jobs/history`: `null` before any job finished.
pub fn history(state: &ApiState, _req: &Request) -> ApiResult {
    let entries = state.jobs.history();
    if entries.is_empty() {
        return Ok(Response::json(&Value::Null));
    }
    Ok(Response::json(&entries))
}

/// `POST /api/jobs/cancel`.
pub fn cancel(state: &ApiState, _req: &Request) -> ApiResult {
    state.jobs.cancel()?;
    Ok(Response::json(&serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------- helpers

fn start(
    state: &ApiState,
    kind: &str,
    options: StartOptions,
    work: crate::jobs::Work,
) -> ApiResult {
    let snapshot = state.jobs.start(kind, options, work)?;
    Ok(Response::json(&snapshot))
}

fn non_empty(text: &str) -> Option<&str> {
    Some(text).filter(|t| !t.is_empty())
}

/// Go's `dashboard.ValidCargueID`.
fn valid_cargue_id(id: &str) -> bool {
    !id.is_empty() && id != ".." && !id.contains(['/', '\\'])
}

fn not_found_cargue(id: &str) -> ApiError {
    ApiError::not_found(format!("{CARGUE_NOT_FOUND}: {id:?}"))
}

fn internal(message: &str, err: &dyn std::fmt::Display) -> ApiError {
    tracing::warn!(error = %err, "{message}");
    ApiError::internal(message)
}

/// Decodes the first JSON value of the body like Go's `Decoder.Decode` (what
/// follows it is ignored) into an object with lower-cased keys, the last
/// duplicate winning. `null` is an empty object. An oversized body is 413 and
/// anything else that is not an object is 400 `cuerpo invalido: <err>`.
fn lenient_body(req: &Request) -> Result<Map<String, Value>, ApiError> {
    if req.body.len() > MAX_JSON_BODY {
        return Err(ApiError::new(413, "cuerpo demasiado grande"));
    }
    let invalid = |err: &str| ApiError::bad_request(format!("cuerpo invalido: {err}"));
    let mut values = serde_json::Deserializer::from_slice(&req.body).into_iter::<Value>();
    let value = match values.next() {
        None => return Err(invalid("EOF")),
        Some(Err(err)) => return Err(invalid(&err.to_string())),
        Some(Ok(value)) => value,
    };
    match value {
        Value::Null => Ok(Map::new()),
        Value::Object(object) => Ok(object
            .into_iter()
            .map(|(key, value)| (key.to_lowercase(), value))
            .collect()),
        other => Err(invalid(&format!(
            "json: cannot unmarshal {} into Go value of type struct",
            json_kind(&other)
        ))),
    }
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// A string field (absent or null is empty).
fn string_field(body: &Map<String, Value>, name: &str) -> Result<String, ApiError> {
    match body.get(&name.to_lowercase()) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(other) => Err(type_error(name, other, "string")),
    }
}

/// An integer field (absent or null is 0).
fn int_field(body: &Map<String, Value>, name: &str) -> Result<i64, ApiError> {
    match body.get(&name.to_lowercase()) {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Number(number)) => number
            .as_i64()
            .ok_or_else(|| type_error(name, &Value::Number(number.clone()), "int")),
        Some(other) => Err(type_error(name, other, "int")),
    }
}

fn type_error(name: &str, value: &Value, want: &str) -> ApiError {
    ApiError::bad_request(format!(
        "cuerpo invalido: json: cannot unmarshal {} into Go struct field .{name} of type {want}",
        json_kind(value)
    ))
}
