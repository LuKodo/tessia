//! State, pipeline and alert derivation shared by the dashboard read models
//! (Go `internal/dashboard`: `native.go`, `summary.go`, `tracking.go`
//! `radicacionSteps`/`artifactStatus`/`mipresColumn`, `dashboard.go`
//! `statusFromCounts`).
//!
//! Everything here is a pure function of the facts the stores return
//! ([`NativeRaw`], [`DatasetRow`]); no function touches the database. The rules
//! are the ones in `odd/reference/ui-api-spec.md` section 3, and the UI's
//! vocabulary (estado names, dots, pipelines, alert codes) is a stored
//! contract: do not rename.

use std::sync::LazyLock;

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use regex::Regex;
use rust_decimal::Decimal;
use serde::{Serialize, Serializer};

use crate::api::{ApiError, ApiResult, ApiState, Request};
use crate::domain::dataset::InvoiceState;
use crate::domain::radicacion::RadStatus;
use crate::phases::radicacion::rules::{
    CODE_AGREEMENT_VALUE_UNAVAILABLE, CODE_INVOICE_VALUE_UNAVAILABLE, CODE_VALIDATION_UNAVAILABLE,
    CODE_VALUE_DIFFERENT, expired_rejection,
};
use crate::stores::dataset::dashboard::{DetailRaw, NativeRaw, PhaseRaw, RadRaw};
use crate::util::text::normalize_key;

// ---- vocabulary ----------------------------------------------------------

pub const ESTADO_TERMINADO: &str = "TERMINADO";
pub const ESTADO_RECHAZADO: &str = "RECHAZADO";
pub const ESTADO_VENCIDO: &str = "VENCIDO";
pub const ESTADO_NOTA_AJUSTE: &str = "NOTA_AJUSTE";
pub const ESTADO_EN_PROCESO: &str = "EN_PROCESO";
/// The estado of an invoice that has native rows but has not progressed.
pub const ESTADO_CARGUE: &str = "CARGUE";
/// Display estado of a prepared invoice with no native row at all.
pub const ESTADO_SIN_INICIAR: &str = "SIN INICIAR";

pub const PIPELINE_NO_APLICA: &str = "no_aplica";
pub const PIPELINE_NO_RADICADO: &str = "no_radicado";
pub const PIPELINE_EN_PROCESO: &str = "en_proceso";
pub const PIPELINE_LISTA: &str = "lista_para_radicar";
pub const PIPELINE_PARCIAL: &str = "radicacion_parcial";
pub const PIPELINE_RADICADA: &str = "radicada";
pub const PIPELINE_RECHAZADA: &str = "rechazada";

/// Every pipeline category (the values the `pipeline` filter accepts).
pub const PIPELINES: [&str; 7] = [
    PIPELINE_NO_APLICA,
    PIPELINE_NO_RADICADO,
    PIPELINE_EN_PROCESO,
    PIPELINE_LISTA,
    PIPELINE_PARCIAL,
    PIPELINE_RADICADA,
    PIPELINE_RECHAZADA,
];

pub const ALERT_FALTA_ZIP: &str = "falta_zip";
pub const ALERT_FALTA_JSON: &str = "falta_json";
pub const ALERT_FALTA_CUV: &str = "falta_cuv";
pub const ALERT_FALTA_CRC: &str = "falta_crc";
pub const ALERT_PENDIENTE_RADICAR: &str = "pendiente_radicar";
pub const ALERT_NOTA_AJUSTE: &str = "nota_ajuste";
pub const ALERT_NOTA_CREDITO: &str = "nota_credito";
pub const ALERT_REQUIERE_AUTORIZACION: &str = "requiere_autorizacion";
pub const ALERT_VENCIDA: &str = "vencida_22_dias";
pub const ALERT_SIN_AUTORIZACION: &str = "sin_autorizacion";

/// Every alert code, in the order of the "Alertas y pendientes" panel.
pub const ALERTS: [&str; 10] = [
    ALERT_FALTA_ZIP,
    ALERT_FALTA_JSON,
    ALERT_FALTA_CUV,
    ALERT_FALTA_CRC,
    ALERT_PENDIENTE_RADICAR,
    ALERT_NOTA_AJUSTE,
    ALERT_NOTA_CREDITO,
    ALERT_REQUIERE_AUTORIZACION,
    ALERT_VENCIDA,
    ALERT_SIN_AUTORIZACION,
];

/// Free text shown next to an invoice is capped at this many characters.
const MAX_DETAIL_CHARS: usize = 600;
/// A list row keeps this many characters of the estado detail.
const LIST_DETAIL_CHARS: usize = 90;

/// `id_mipres` of invoices whose subplan is not EVENTO NO PBS.
pub const NOT_APPLICABLE: &str = "No aplica";

// ---- dots ----------------------------------------------------------------

/// State of an artifact or of a filing step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dot {
    Ok,
    Error,
    Pending,
    Running,
}

impl Dot {
    pub fn as_str(self) -> &'static str {
        match self {
            Dot::Ok => "ok",
            Dot::Error => "error",
            Dot::Pending => "pendiente",
            Dot::Running => "en_curso",
        }
    }
}

impl Serialize for Dot {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// One artifact dot from the `proceso_estado.estado` of its phase: `DONE` is ok,
/// `FAILED` and `REVISION_MANUAL` are errors, anything else is pending.
pub fn artifact_dot(state: Option<&str>) -> Dot {
    match state {
        Some("DONE") => Dot::Ok,
        Some("FAILED" | "REVISION_MANUAL") => Dot::Error,
        _ => Dot::Pending,
    }
}

/// Is the filing row `CONFIRMED` at the last step (Go `State.Done`)?
fn rad_done(rad: &RadRaw) -> bool {
    rad.step == 4 && rad.status == RadStatus::Confirmed.as_str()
}

/// Go `State.Terminal` minus the confirmed case: the statuses no step can leave.
fn rad_terminal(status: &str) -> bool {
    [
        RadStatus::AdjustmentNote,
        RadStatus::CreditNote,
        RadStatus::Blocked,
        RadStatus::Rejected,
    ]
    .iter()
    .any(|s| s.as_str() == status)
}

/// The four filing-step boxes: a step behind the row's current one is ok; the
/// current step is running, confirmed or failed (terminal); later ones pending.
pub fn filing_dots(rad: Option<&RadRaw>) -> [Dot; 4] {
    let mut out = [Dot::Pending; 4];
    let Some(rad) = rad else { return out };
    for (k, dot) in out.iter_mut().enumerate() {
        let step = k as i32 + 1;
        *dot = if step > rad.step {
            Dot::Pending
        } else if step < rad.step || rad.status == RadStatus::Confirmed.as_str() {
            Dot::Ok
        } else if rad_terminal(&rad.status) {
            Dot::Error
        } else {
            Dot::Running
        };
    }
    out
}

/// A `BLOCKED` filing whose detail carries a value-validation code.
fn value_blocked(rad: &RadRaw) -> bool {
    if rad.status != RadStatus::Blocked.as_str() {
        return false;
    }
    let text = rad.detail.as_deref().unwrap_or("").to_uppercase();
    [
        CODE_VALUE_DIFFERENT,
        CODE_INVOICE_VALUE_UNAVAILABLE,
        CODE_AGREEMENT_VALUE_UNAVAILABLE,
        CODE_VALIDATION_UNAVAILABLE,
    ]
    .iter()
    .any(|code| text.contains(code))
}

fn rad_expired(rad: &RadRaw) -> bool {
    rad.detail.as_deref().is_some_and(expired_rejection)
}

/// The estado derived from the native rows (Go `Progress.Estado`), first match
/// wins: a fully filed invoice is `TERMINADO`; a credit note is `VENCIDO` when
/// Mutual Ser rejected it with `RFFE022` and `RECHAZADO` otherwise; an
/// adjustment note is `NOTA_AJUSTE`; a `BLOCKED` filing carrying a value block
/// code is `RECHAZADO`; any generated artifact is `EN_PROCESO`; else `CARGUE`.
pub fn estado_of(native: &NativeRaw) -> &'static str {
    if let Some(rad) = &native.rad {
        let status = rad.status.as_str();
        if rad_done(rad) {
            return ESTADO_TERMINADO;
        } else if status == RadStatus::CreditNote.as_str() {
            return if rad_expired(rad) {
                ESTADO_VENCIDO
            } else {
                ESTADO_RECHAZADO
            };
        } else if status == RadStatus::AdjustmentNote.as_str() {
            return ESTADO_NOTA_AJUSTE;
        } else if value_blocked(rad) {
            return ESTADO_RECHAZADO;
        }
    }
    if native
        .artifacts
        .iter()
        .any(|a| a.as_deref() == Some("DONE"))
    {
        return ESTADO_EN_PROCESO;
    }
    ESTADO_CARGUE
}

// ---- classification ------------------------------------------------------

/// Everything derived for one row of the tracking table (Go `enrichRows` +
/// `classify`).
#[derive(Debug, Clone)]
pub struct Derived {
    pub state: InvoiceState,
    /// Set only for a prepared invoice that has native rows.
    pub estado: Option<&'static str>,
    pub artifacts: [Dot; 4],
    pub filing: [Dot; 4],
    pub expired: bool,
    pub note_adjustment: bool,
    pub note_credit: bool,
    pub pipeline: &'static str,
    pub alerts: Vec<&'static str>,
    /// Latest update over the invoice's native rows.
    pub updated_at: Option<DateTime<Utc>>,
}

/// Derives the row. `native` is the invoice's native facts (`None` when it has
/// no row in either table); a row that is not prepared never reads them.
pub fn derive(state: InvoiceState, authorization: &str, native: Option<&NativeRaw>) -> Derived {
    let mut d = Derived {
        state,
        estado: None,
        artifacts: [Dot::Pending; 4],
        filing: [Dot::Pending; 4],
        expired: false,
        note_adjustment: false,
        note_credit: false,
        pipeline: PIPELINE_NO_APLICA,
        alerts: Vec::new(),
        updated_at: None,
    };
    if state != InvoiceState::Prepared {
        return d;
    }
    if let Some(n) = native {
        d.estado = Some(estado_of(n));
        d.updated_at = latest(n);
        for (dot, phase) in d.artifacts.iter_mut().zip(&n.artifacts) {
            *dot = artifact_dot(phase.as_deref());
        }
        if let Some(rad) = &n.rad {
            let status = rad.status.as_str();
            d.note_adjustment = status == RadStatus::AdjustmentNote.as_str();
            d.note_credit = status == RadStatus::CreditNote.as_str();
            d.expired = rad_expired(rad);
        }
        d.filing = filing_dots(n.rad.as_ref());
    }
    classify(&mut d, authorization);
    d
}

fn latest(n: &NativeRaw) -> Option<DateTime<Utc>> {
    n.rad
        .as_ref()
        .map(|r| r.updated_at)
        .into_iter()
        .chain(n.phase_updated)
        .max()
}

/// Filing step that uploads each artifact (ZIP, JSON, CUV, CRC): a filed step
/// proves its artifact existed even without a flag. Indexes into `filing`.
const ARTIFACT_STEP: [usize; 4] = [0, 2, 1, 3];
const ARTIFACT_ALERTS: [&str; 4] = [
    ALERT_FALTA_ZIP,
    ALERT_FALTA_JSON,
    ALERT_FALTA_CUV,
    ALERT_FALTA_CRC,
];

fn count(dots: &[Dot; 4], want: Dot) -> usize {
    dots.iter().filter(|d| **d == want).count()
}

/// Sets the pipeline category and the alerts of a prepared row (Go `classify`).
fn classify(d: &mut Derived, authorization: &str) {
    let filed = count(&d.filing, Dot::Ok);
    let generated = count(&d.artifacts, Dot::Ok);
    d.pipeline = if filed == 4 {
        PIPELINE_RADICADA
    } else if count(&d.filing, Dot::Error) > 0
        || d.expired
        || matches!(d.estado, Some(ESTADO_RECHAZADO | ESTADO_VENCIDO))
    {
        PIPELINE_RECHAZADA
    } else if filed > 0 || count(&d.filing, Dot::Running) > 0 {
        PIPELINE_PARCIAL
    } else if generated == 4 {
        PIPELINE_LISTA
    } else if generated > 0 || count(&d.artifacts, Dot::Error) > 0 {
        PIPELINE_EN_PROCESO
    } else {
        PIPELINE_NO_RADICADO
    };

    let mut alerts = Vec::new();
    for (k, alert) in ARTIFACT_ALERTS.iter().enumerate() {
        if d.artifacts[k] != Dot::Ok && d.filing[ARTIFACT_STEP[k]] != Dot::Ok {
            alerts.push(*alert);
        }
    }
    if d.pipeline != PIPELINE_RADICADA && d.pipeline != PIPELINE_RECHAZADA {
        alerts.push(ALERT_PENDIENTE_RADICAR);
    }
    if d.note_adjustment {
        alerts.push(ALERT_NOTA_AJUSTE);
    }
    if d.note_credit {
        alerts.push(ALERT_NOTA_CREDITO);
    }
    // "Requiere autorización" and "Sin autorización" share one criterion.
    let no_authorization = authorization.trim().is_empty();
    if no_authorization {
        alerts.push(ALERT_REQUIERE_AUTORIZACION);
    }
    if d.expired {
        alerts.push(ALERT_VENCIDA);
    }
    if no_authorization {
        alerts.push(ALERT_SIN_AUTORIZACION);
    }
    d.alerts = alerts;
}

/// The estado the table shows (the UI's `estadoOf`): the derived estado, `SIN
/// INICIAR` for a prepared row that has none, else the outcome in upper case.
pub fn display_estado(d: &Derived) -> &'static str {
    match (d.estado, d.state) {
        (Some(estado), _) => estado,
        (None, InvoiceState::Prepared) => ESTADO_SIN_INICIAR,
        (None, InvoiceState::Excluded) => "EXCLUIDA",
        (None, InvoiceState::ManualReview) => "REVISION_MANUAL",
    }
}

/// The `resultado` of a stored state.
pub fn resultado(state: InvoiceState) -> &'static str {
    match state {
        InvoiceState::Prepared => "preparada",
        InvoiceState::Excluded => "excluida",
        InvoiceState::ManualReview => "revision_manual",
    }
}

/// Parses `dataset_factura.estado`; anything unknown counts as manual review so
/// no invoice is ever lost from a total (Go `summarizeCargues`).
pub fn parse_state(estado: &str) -> InvoiceState {
    InvoiceState::parse(estado).unwrap_or(InvoiceState::ManualReview)
}

/// Cargue-level status from the derived estados of its prepared invoices (Go
/// `statusFromCounts`): `terminados == total` is `COMPLETADO`; terminados plus
/// the incident estados equal to total is `CON_INCIDENCIAS`; else `PROCESANDO`.
pub fn status_from_counts(total: usize, terminated: usize, incidents: usize) -> &'static str {
    if terminated == total {
        "COMPLETADO"
    } else if terminated + incidents == total {
        "CON_INCIDENCIAS"
    } else {
        "PROCESANDO"
    }
}

/// Status of a cargue with no prepared invoice.
pub fn status_without_invoices(excluded: i64, manual_review: i64) -> &'static str {
    if excluded + manual_review > 0 {
        "CON_INCIDENCIAS"
    } else {
        "SIN_FACTURAS"
    }
}

/// Is `estado` one of the incident estados of [`status_from_counts`]?
pub fn is_incident(estado: &str) -> bool {
    matches!(
        estado,
        ESTADO_RECHAZADO | ESTADO_VENCIDO | ESTADO_NOTA_AJUSTE
    )
}

/// The "ID MIPRES" cell: the verified delivery id for EVENTO NO PBS (empty when
/// none was verified), `No aplica` for any other known subplan, empty when the
/// subplan is unknown (it is never guessed).
pub fn mipres_column(subplan: &str, delivery_id: &str) -> String {
    if subplan.trim().is_empty() {
        String::new()
    } else if normalize_key(subplan) == "EVENTO NO PBS" {
        delivery_id.to_owned()
    } else {
        NOT_APPLICABLE.to_owned()
    }
}

// ---- free texts ----------------------------------------------------------

/// Collapses whitespace runs into single spaces and trims (Go `strings.Fields`).
pub fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn detail_text(text: &str) -> String {
    truncate_chars(&collapse(text), MAX_DETAIL_CHARS)
}

/// The free text behind an invoice's estado (Go `Progress.Detail`): the last
/// radicación detail when it has one, otherwise the reason of the most recently
/// updated phase (ties go to the later phase of the pipeline). Whitespace
/// collapsed and capped at 600 characters; empty when no row has text.
pub fn progress_detail(detail: &DetailRaw) -> String {
    if let Some(text) = detail.rad_detail.as_deref() {
        let text = detail_text(text);
        if !text.is_empty() {
            return text;
        }
    }
    let mut phases: Vec<&PhaseRaw> = detail.phases.iter().collect();
    phases.sort_by_key(|p| (p.updated_at, phase_rank(&p.fase)));
    phases
        .iter()
        .rev()
        .map(|p| detail_text(&p.motivo))
        .find(|t| !t.is_empty())
        .unwrap_or_default()
}

/// Position of a phase in the per-invoice pipeline (`proceso run` order).
fn phase_rank(fase: &str) -> usize {
    [
        "convenio",
        "derechos",
        "thefactory",
        "fev_ajuste",
        "fev_valor",
        "conciliacion",
        "rips",
        "cuv",
        "soporte",
    ]
    .iter()
    .position(|p| *p == fase)
    .unwrap_or(usize::MAX)
}

/// A list row carries the text itself up to 90 characters, else its first 90
/// plus an ellipsis (Go `ShortDetail`).
pub fn short_detail(text: &str) -> String {
    match text.char_indices().nth(LIST_DETAIL_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

static REASON_CODE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)^([A-Za-z][A-Za-z0-9_]*):\s+(.+)$")
        .unwrap_or_else(|_| unreachable!("constant regex"))
});

/// Separates a leading `CODE: text` code from the human text.
pub fn split_reason(reason: &str) -> (String, String) {
    let reason = reason.trim();
    match REASON_CODE_RE.captures(reason) {
        Some(m) => (m[1].to_owned(), m[2].trim().to_owned()),
        None => (String::new(), reason.to_owned()),
    }
}

const BLOCKED_BY_TARIFF: &str = "Bloqueada por tarifa: un precio del XML supera la nota técnica. Requiere nota crédito y refacturación.";
const BLOCKED_BY_AMOUNT: &str =
    "Bloqueada por monto: el total reconstruido supera el máximo permitido por factura.";

/// Replaces the radicación's generic value-mismatch detail with the real cause
/// when `conciliar` already ruled the invoice out for good (Go
/// `applyConciliarBlocks`). Any other detail is left alone.
pub fn apply_conciliar_block(detail: &mut String, rows: Option<&DetailRaw>) {
    if !detail.contains(CODE_VALUE_DIFFERENT) {
        return;
    }
    let Some(rows) = rows else { return };
    let Some(state) = rows.phases.iter().find(|p| p.fase == "conciliacion") else {
        return;
    };
    if state.estado != "EXCLUDED" {
        return;
    }
    match split_reason(&state.motivo).0.as_str() {
        "BLOQUEADA_TARIFA" => *detail = BLOCKED_BY_TARIFF.to_owned(),
        "BLOQUEADA_MONTO" => *detail = BLOCKED_BY_AMOUNT.to_owned(),
        _ => {}
    }
}

// ---- formatting and input helpers ------------------------------------------

/// A decimal as a plain string without trailing zeros (`110.00` is `110`).
pub fn plain_amount(value: Decimal) -> String {
    value.normalize().to_string()
}

/// RFC 3339 in UTC, with fractional seconds only when present.
pub fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// The cursor/sort text of a timestamp: fixed width, so text order is time order.
pub fn sortable_time(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M:%S%.6f").to_string()
}

/// A strict `YYYY-MM-DD` (Go `time.Parse("2006-01-02")`).
pub fn parse_day(text: &str) -> Option<NaiveDate> {
    let b = text.as_bytes();
    let shaped = b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        });
    if !shaped {
        return None;
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

/// Go's `%q` for the strings the messages quote.
pub fn quoted(text: &str) -> String {
    format!("{text:?}")
}

/// Go's `ValidCargueID`: a single path component, never a traversal.
pub fn valid_cargue_id(id: &str) -> bool {
    !id.is_empty() && id != ".." && !id.contains(['/', '\\'])
}

// ---- cache seam ------------------------------------------------------------

/// The one seam to the shared 2.5 s answer cache (`crate::api::cache`,
/// `Cache::get_or_compute`) that `cargues`, `consulta` and `resumen` go through.
///
/// The cache lands in another branch; until the parent points this function at
/// it, the answer is computed directly. `key` identifies the answer (route plus
/// the canonical query, see [`cache_key`]); only a 200 may be cached.
pub fn cached(_state: &ApiState, _key: &str, compute: impl FnOnce() -> ApiResult) -> ApiResult {
    compute()
}

/// Cache key of a request: the route plus its query pairs in sorted order.
pub fn cache_key(route: &str, req: &Request) -> String {
    let mut pairs: Vec<String> = req.query.iter().map(|(k, v)| format!("{k}={v}")).collect();
    pairs.sort();
    format!("{route}?{}", pairs.join("&"))
}

/// The generic 500 of a read route (never a raw database error).
pub fn internal(message: &str) -> ApiError {
    ApiError::internal(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 10, h, 0, 0)
            .single()
            .expect("time")
    }

    fn rad(step: i32, status: &str, detail: Option<&str>) -> RadRaw {
        RadRaw {
            step,
            status: status.into(),
            updated_at: at(10),
            detail: detail.map(str::to_owned),
        }
    }

    fn native(rad: Option<RadRaw>, artifacts: [Option<&str>; 4]) -> NativeRaw {
        NativeRaw {
            rad,
            artifacts: artifacts.map(|a| a.map(str::to_owned)),
            has_phase_rows: true,
            phase_updated: Some(at(9)),
        }
    }

    const NONE4: [Option<&str>; 4] = [None; 4];

    fn row(native: Option<&NativeRaw>, auth: &str) -> Derived {
        derive(InvoiceState::Prepared, auth, native)
    }

    #[test]
    fn filing_dots_follow_the_current_step_and_status() {
        use Dot::*;
        assert_eq!(filing_dots(None), [Pending; 4]);
        assert_eq!(
            filing_dots(Some(&rad(4, "CONFIRMED", None))),
            [Ok, Ok, Ok, Ok]
        );
        assert_eq!(
            filing_dots(Some(&rad(2, "SENT", None))),
            [Ok, Running, Pending, Pending]
        );
        assert_eq!(
            filing_dots(Some(&rad(3, "ADJUSTMENT_NOTE", None))),
            [Ok, Ok, Error, Pending]
        );
        assert_eq!(
            filing_dots(Some(&rad(1, "REJECTED", None))),
            [Error, Pending, Pending, Pending]
        );
        // Confirmed below the last step: that step is ok, later ones pending.
        assert_eq!(
            filing_dots(Some(&rad(2, "CONFIRMED", None))),
            [Ok, Ok, Pending, Pending]
        );
        assert_eq!(
            filing_dots(Some(&rad(1, "PENDING", None))),
            [Running, Pending, Pending, Pending]
        );
    }

    #[test]
    fn artifact_dots_map_the_phase_state() {
        assert_eq!(artifact_dot(Some("DONE")), Dot::Ok);
        assert_eq!(artifact_dot(Some("FAILED")), Dot::Error);
        assert_eq!(artifact_dot(Some("REVISION_MANUAL")), Dot::Error);
        assert_eq!(artifact_dot(Some("EN_PROGRESO")), Dot::Pending);
        assert_eq!(artifact_dot(Some("EXCLUDED")), Dot::Pending);
        assert_eq!(artifact_dot(None), Dot::Pending);
    }

    #[test]
    fn the_estado_is_the_first_matching_rule() {
        let done = native(Some(rad(4, "CONFIRMED", None)), NONE4);
        assert_eq!(estado_of(&done), "TERMINADO");
        let expired = native(
            Some(rad(1, "CREDIT_NOTE", Some("RFFE022: superó los 22 días"))),
            NONE4,
        );
        assert_eq!(estado_of(&expired), "VENCIDO");
        let phrase_only = native(
            Some(rad(1, "CREDIT_NOTE", Some("supera los 22 días hábiles"))),
            NONE4,
        );
        assert_eq!(estado_of(&phrase_only), "RECHAZADO");
        let adjustment = native(Some(rad(3, "ADJUSTMENT_NOTE", None)), NONE4);
        assert_eq!(estado_of(&adjustment), "NOTA_AJUSTE");
        let blocked = native(
            Some(rad(
                1,
                "BLOCKED",
                Some("Paso 1 bloqueado: VALOR_FACTURA_DIFERENTE"),
            )),
            NONE4,
        );
        assert_eq!(estado_of(&blocked), "RECHAZADO");
        let blocked_other = native(Some(rad(1, "BLOCKED", Some("otra cosa"))), NONE4);
        assert_eq!(estado_of(&blocked_other), "CARGUE");
        let generated = native(Some(rad(2, "SENT", None)), [Some("DONE"), None, None, None]);
        assert_eq!(estado_of(&generated), "EN_PROCESO");
        let untouched = native(None, [None, Some("FAILED"), None, None]);
        assert_eq!(estado_of(&untouched), "CARGUE");
    }

    #[test]
    fn a_filed_invoice_is_radicada_and_raises_no_artifact_alert() {
        let n = native(Some(rad(4, "CONFIRMED", None)), NONE4);
        let d = row(Some(&n), "AUT");
        assert_eq!(d.pipeline, "radicada");
        assert!(d.alerts.is_empty());
        assert_eq!(d.estado, Some("TERMINADO"));
        assert_eq!(d.updated_at, Some(at(10)));
    }

    #[test]
    fn pipeline_categories_follow_the_classification_order() {
        let none = row(None, "A");
        assert_eq!((none.pipeline, none.estado), ("no_radicado", None));
        assert_eq!(display_estado(&none), "SIN INICIAR");

        let some = native(None, [Some("DONE"), None, None, None]);
        assert_eq!(row(Some(&some), "A").pipeline, "en_proceso");
        let failed = native(None, [None, Some("FAILED"), None, None]);
        assert_eq!(row(Some(&failed), "A").pipeline, "en_proceso");
        let all = native(None, [Some("DONE"); 4]);
        assert_eq!(row(Some(&all), "A").pipeline, "lista_para_radicar");
        let partial = native(Some(rad(2, "SENT", None)), NONE4);
        assert_eq!(row(Some(&partial), "A").pipeline, "radicacion_parcial");
        let rejected = native(Some(rad(3, "ADJUSTMENT_NOTE", None)), NONE4);
        assert_eq!(row(Some(&rejected), "A").pipeline, "rechazada");
        let expired = native(Some(rad(2, "SENT", Some("rffe022"))), NONE4);
        let d = row(Some(&expired), "A");
        assert_eq!(d.pipeline, "rechazada");
        assert!(d.expired);
    }

    #[test]
    fn alerts_come_in_panel_order() {
        // Golden FVS5: one artifact ok, filing step 1 failed as a credit note
        // rejected with RFFE022, no authorization alert (it has one).
        let n = native(
            Some(rad(
                1,
                "CREDIT_NOTE",
                Some("RFFE022: superó los 22 días hábiles"),
            )),
            [Some("DONE"), None, None, None],
        );
        let d = row(Some(&n), "AUT");
        assert_eq!(d.estado, Some("VENCIDO"));
        assert_eq!(
            d.alerts,
            [
                "falta_json",
                "falta_cuv",
                "falta_crc",
                "nota_credito",
                "vencida_22_dias"
            ]
        );
        // Without native rows and without authorization every alert but the
        // note and expiry ones fires, the two authorization codes together.
        let d = row(None, "  ");
        assert_eq!(
            d.alerts,
            [
                "falta_zip",
                "falta_json",
                "falta_cuv",
                "falta_crc",
                "pendiente_radicar",
                "requiere_autorizacion",
                "sin_autorizacion"
            ]
        );
    }

    #[test]
    fn an_artifact_alert_is_cleared_by_its_filing_step() {
        // zip<->step 1, cuv<->step 2, json<->step 3, crc<->step 4.
        // Golden FVS2: step 2 is running, so only the zip alert is cleared.
        let running = native(Some(rad(2, "SENT", None)), NONE4);
        assert_eq!(
            row(Some(&running), "A").alerts,
            ["falta_json", "falta_cuv", "falta_crc", "pendiente_radicar"]
        );
        // Step 3 running: steps 1 and 2 are ok, so zip and cuv are cleared.
        let third = native(Some(rad(3, "SENT", None)), NONE4);
        assert_eq!(
            row(Some(&third), "A").alerts,
            ["falta_json", "falta_crc", "pendiente_radicar"]
        );
        // A generated artifact clears its own alert too.
        let json = native(None, [None, Some("DONE"), None, None]);
        assert_eq!(
            row(Some(&json), "A").alerts,
            ["falta_zip", "falta_cuv", "falta_crc", "pendiente_radicar"]
        );
    }

    #[test]
    fn rows_that_are_not_prepared_never_read_native_state() {
        let n = native(Some(rad(4, "CONFIRMED", None)), [Some("DONE"); 4]);
        let d = derive(InvoiceState::Excluded, "", Some(&n));
        assert_eq!(d.pipeline, "no_aplica");
        assert!(d.alerts.is_empty());
        assert_eq!(d.estado, None);
        assert_eq!(display_estado(&d), "EXCLUIDA");
        let m = derive(InvoiceState::ManualReview, "", None);
        assert_eq!(display_estado(&m), "REVISION_MANUAL");
    }

    #[test]
    fn cargue_status_rolls_up_the_estados() {
        assert_eq!(status_from_counts(3, 3, 0), "COMPLETADO");
        assert_eq!(status_from_counts(3, 1, 2), "CON_INCIDENCIAS");
        assert_eq!(status_from_counts(3, 1, 1), "PROCESANDO");
        assert_eq!(status_without_invoices(0, 0), "SIN_FACTURAS");
        assert_eq!(status_without_invoices(1, 0), "CON_INCIDENCIAS");
        assert!(is_incident("VENCIDO") && !is_incident("CARGUE"));
    }

    #[test]
    fn mipres_follows_the_subplan() {
        assert_eq!(mipres_column("  ", "1"), "");
        assert_eq!(mipres_column("evento  no pbs", "111, 222"), "111, 222");
        assert_eq!(mipres_column("EVENTO NO PBS", ""), "");
        assert_eq!(mipres_column("EVENTO POS", "1"), "No aplica");
    }

    fn phase(fase: &str, motivo: &str, h: u32) -> PhaseRaw {
        PhaseRaw {
            num_factura: "F".into(),
            fase: fase.into(),
            estado: "DONE".into(),
            motivo: motivo.into(),
            updated_at: at(h),
        }
    }

    #[test]
    fn the_detail_prefers_the_filing_text_then_the_newest_phase_reason() {
        let both = DetailRaw {
            rad_detail: Some("  Radicada \n completa ".into()),
            phases: vec![phase("rips", "x", 9)],
        };
        assert_eq!(progress_detail(&both), "Radicada completa");
        let blank_rad = DetailRaw {
            rad_detail: Some("   ".into()),
            phases: vec![
                phase("rips", "viejo", 8),
                phase("cuv", "nuevo  texto", 9),
                phase("soporte", "", 10),
            ],
        };
        assert_eq!(progress_detail(&blank_rad), "nuevo texto");
        let tie = DetailRaw {
            rad_detail: None,
            phases: vec![phase("rips", "a", 9), phase("cuv", "b", 9)],
        };
        assert_eq!(progress_detail(&tie), "b", "ties go to the later phase");
        assert_eq!(progress_detail(&DetailRaw::default()), "");
        let long = DetailRaw {
            rad_detail: Some("x".repeat(700)),
            phases: vec![],
        };
        assert_eq!(progress_detail(&long).chars().count(), 600);
    }

    #[test]
    fn list_rows_cut_the_detail_after_ninety_characters() {
        let exact = "á".repeat(90);
        assert_eq!(short_detail(&exact), exact);
        let long = "á".repeat(91);
        assert_eq!(short_detail(&long), format!("{}…", "á".repeat(90)));
        assert_eq!(short_detail(""), "");
    }

    #[test]
    fn reasons_split_a_leading_code() {
        assert_eq!(
            split_reason(" BLOQUEADA_TARIFA: precio  alto "),
            ("BLOQUEADA_TARIFA".into(), "precio  alto".into())
        );
        assert_eq!(
            split_reason("sin codigo"),
            (String::new(), "sin codigo".into())
        );
        assert_eq!(split_reason("CODIGO:sin espacio").0, "");
    }

    #[test]
    fn a_permanent_conciliar_block_replaces_the_value_mismatch_text() {
        let blocked = |motivo: &str, estado: &str| DetailRaw {
            rad_detail: None,
            phases: vec![PhaseRaw {
                estado: estado.into(),
                ..phase("conciliacion", motivo, 9)
            }],
        };
        let generic = "Paso 1 bloqueado: VALOR_FACTURA_DIFERENTE: el total no coincide";
        let mut text = generic.to_owned();
        apply_conciliar_block(&mut text, Some(&blocked("BLOQUEADA_TARIFA: x", "EXCLUDED")));
        assert!(text.starts_with("Bloqueada por tarifa: un precio del XML supera"));
        let mut text = generic.to_owned();
        apply_conciliar_block(&mut text, Some(&blocked("BLOQUEADA_MONTO: x", "EXCLUDED")));
        assert!(text.starts_with("Bloqueada por monto:"));
        for (rows, detail) in [
            (Some(blocked("BLOQUEADA_MONTO: x", "DONE")), generic),
            (Some(blocked("OTRO: x", "EXCLUDED")), generic),
            (
                Some(blocked("BLOQUEADA_MONTO: x", "EXCLUDED")),
                "otro texto",
            ),
            (None, generic),
        ] {
            let mut text = detail.to_owned();
            apply_conciliar_block(&mut text, rows.as_ref());
            assert_eq!(text, detail);
        }
    }

    #[test]
    fn input_helpers_are_strict() {
        assert!(parse_day("2026-09-30").is_some());
        for bad in [
            "2026-9-30",
            "2026-02-30",
            "26-09-30",
            "2026/09/30",
            " 2026-09-30",
            "",
        ] {
            assert!(parse_day(bad).is_none(), "{bad:?}");
        }
        assert!(valid_cargue_id(".") && valid_cargue_id("CargueA"));
        assert!(!valid_cargue_id("") && !valid_cargue_id("..") && !valid_cargue_id("a/b"));
        assert!(!valid_cargue_id("a\\b"));
        assert_eq!(quoted("x\"y"), "\"x\\\"y\"");
        assert_eq!(plain_amount("110.00".parse().expect("dec")), "110");
        assert_eq!(plain_amount("1234.50".parse().expect("dec")), "1234.5");
        assert_eq!(plain_amount(Decimal::ZERO), "0");
        assert_eq!(rfc3339(at(8)), "2026-09-10T08:00:00Z");
    }

    #[test]
    fn cache_keys_ignore_the_order_of_the_query() {
        let a = Request::new("GET", "/x")
            .with_query("b", "2")
            .with_query("a", "1");
        let b = Request::new("GET", "/x")
            .with_query("a", "1")
            .with_query("b", "2");
        assert_eq!(cache_key("consulta", &a), cache_key("consulta", &b));
        assert_ne!(cache_key("consulta", &a), cache_key("resumen", &a));
    }
}
