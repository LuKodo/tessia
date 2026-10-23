//! `GET /api/cargues/{id}/informe?formato=json|csv`: the invoices of one
//! cargue that stopped and need a person (Go `internal/dashboard/informe.go`
//! and `cmd_serve_informe.go`). Built on demand from the stored dataset and
//! `proceso_estado`, never stored.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{DateTime, SecondsFormat, Utc};
use regex::Regex;
use serde::Serialize;

use crate::api::csv::{CsvWriter, download, safe_file_name};
use crate::api::{
    ApiState,
    errors::{ApiError, ApiResult},
    request::{Request, Response},
};
use crate::domain::dataset::InvoiceState;
use crate::domain::{Phase, PhaseState, normalize_invoice};
use crate::stores::proceso::{ProcesoState, ProcesoStore};
use crate::stores::radicacion::{DatasetEntry, ReportStore};

const CATEGORY_MANUAL_REVIEW: &str = "revision_manual";
const CATEGORY_FAILURE: &str = "falla";
const CATEGORY_EXCLUDED: &str = "excluida";

/// The report's phase order (also the sort order inside a category).
const REPORT_PHASES: [Phase; 9] = Phase::ALL;

const READ_FAILED: &str = "no se pudo leer el informe de revisión manual";

/// `(?s)^([A-Za-z][A-Za-z0-9_]*):\s+(.+)$` with Go's ASCII `\s`.
static REASON_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)^([A-Za-z][A-Za-z0-9_]*):[\t\n\x0C\r ]+(.+)$")
        .unwrap_or_else(|_| unreachable!("constant regex"))
});

/// A cargue id is one path component: not empty, not `..`, no `/` or `\`
/// (Go `ValidCargueID`).
pub(crate) fn valid_cargue_id(id: &str) -> bool {
    !id.is_empty() && id != ".." && !id.contains(['/', '\\'])
}

/// Go's `%q` for the ids that can reach a message.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// 404 `dashboard: cargue not found: "<id>"` (`invalid id` for a bad one).
pub(crate) fn cargue_not_found(id: &str) -> ApiError {
    if valid_cargue_id(id) {
        ApiError::not_found(format!("dashboard: cargue not found: {}", quoted(id)))
    } else {
        ApiError::not_found(format!(
            "dashboard: cargue not found: invalid id {}",
            quoted(id)
        ))
    }
}

/// RFC 3339 UTC without fractions (Go `time.RFC3339`).
pub(crate) fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ReportItem {
    factura: String,
    fase: String,
    categoria: &'static str,
    /// Leading code of a `CODE: text` reason, else empty.
    codigo: String,
    motivo: String,
    /// RFC 3339 UTC; empty for entries only known from the dataset.
    actualizado_en: String,
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
struct Counts {
    revision_manual: usize,
    excluidas: usize,
    fallas: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct Report {
    cargue: String,
    generado_en: String,
    /// True when the persisted states could not be read.
    incompleto: bool,
    conteos: Counts,
    items: Vec<ReportItem>,
}

/// Separates a leading `CODE: text` code from the human text.
fn split_reason(reason: &str) -> (String, String) {
    let text = reason.trim();
    match REASON_CODE.captures(text) {
        Some(caps) => (caps[1].to_owned(), caps[2].trim().to_owned()),
        None => (String::new(), text.to_owned()),
    }
}

fn category_of(state: PhaseState) -> Option<&'static str> {
    match state {
        PhaseState::RevisionManual => Some(CATEGORY_MANUAL_REVIEW),
        PhaseState::Failed => Some(CATEGORY_FAILURE),
        PhaseState::Excluded => Some(CATEGORY_EXCLUDED),
        // DONE and EN_PROGRESO never need a person.
        PhaseState::Done | PhaseState::EnProgreso => None,
    }
}

fn category_rank(category: &str) -> u8 {
    match category {
        CATEGORY_MANUAL_REVIEW => 0,
        CATEGORY_FAILURE => 1,
        _ => 2,
    }
}

fn phase_rank(phase: &str) -> usize {
    REPORT_PHASES
        .iter()
        .position(|p| p.as_str() == phase)
        .unwrap_or(REPORT_PHASES.len())
}

/// The `BLOQUEADA_*` code of a conciliacion state that ruled the invoice out
/// for good (price above the nota técnica, or amount above the cap).
fn permanent_block(state: Option<&ProcesoState>) -> bool {
    let Some(state) = state.filter(|s| s.state == PhaseState::Excluded) else {
        return false;
    };
    matches!(
        split_reason(&state.motivo).0.as_str(),
        "BLOQUEADA_TARIFA" | "BLOQUEADA_MONTO"
    )
}

/// Conciliar ended the invoice on a permanent block at or after the given
/// `fev_valor` check: that check's difference is superseded, so it must not
/// look like something to reconcile again.
fn blocked_after(conciliacion: Option<&ProcesoState>, fev_valor: &ProcesoState) -> bool {
    permanent_block(conciliacion)
        && conciliacion.is_some_and(|c| c.updated_at >= fev_valor.updated_at)
}

type PhaseStates = HashMap<Phase, HashMap<String, ProcesoState>>;

fn dataset_item(entry: &DatasetEntry) -> Option<ReportItem> {
    let (category, code, reason) = match InvoiceState::parse(&entry.state)? {
        InvoiceState::Prepared => return None,
        InvoiceState::Excluded => {
            let (code, text) = split_reason(&entry.reason);
            (CATEGORY_EXCLUDED, code, text)
        }
        InvoiceState::ManualReview => (
            CATEGORY_MANUAL_REVIEW,
            entry.code.clone(),
            entry.reason.trim().to_owned(),
        ),
    };
    Some(ReportItem {
        factura: entry.invoice.clone(),
        fase: Phase::Convenio.as_str().to_owned(),
        categoria: category,
        codigo: code,
        motivo: reason,
        actualizado_en: String::new(),
    })
}

/// Lists every non-DONE, non-EN_PROGRESO phase state of the dataset's invoices,
/// plus the convenio-stage stops recorded in the dataset. `states == None`
/// means the stored states could not be read: only the dataset entries are
/// listed and the report is flagged `incompleto`.
fn build_report(
    entries: &[DatasetEntry],
    states: Option<&PhaseStates>,
    cargue: &str,
    now: DateTime<Utc>,
) -> Report {
    let mut display: HashMap<String, &str> = HashMap::new();
    for entry in entries {
        display
            .entry(normalize_invoice(&entry.invoice))
            .or_insert(entry.invoice.as_str());
    }
    // Convenio-stage entries by normalized invoice; a later entry replaces an
    // earlier one of the same invoice (manual review after excluded).
    let mut dataset_items: Vec<(String, ReportItem)> = Vec::new();
    for entry in entries {
        let Some(item) = dataset_item(entry) else {
            continue;
        };
        let key = normalize_invoice(&entry.invoice);
        match dataset_items.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = item,
            None => dataset_items.push((key, item)),
        }
    }

    let state_of =
        |phase: Phase, key: &str| -> Option<&ProcesoState> { states?.get(&phase)?.get(key) };
    let mut items: Vec<ReportItem> = Vec::new();
    for (key, mut item) in dataset_items.iter().cloned() {
        let convenio = state_of(Phase::Convenio, &key);
        if convenio.is_some_and(|s| s.state == PhaseState::Done) {
            continue; // fixed after the dataset was written
        }
        if let Some(state) = convenio {
            item.actualizado_en = rfc3339(state.updated_at);
        }
        items.push(item);
    }
    let in_dataset = |key: &str| dataset_items.iter().any(|(k, _)| k == key);
    for phase in REPORT_PHASES {
        let Some(by_invoice) = states.and_then(|s| s.get(&phase)) else {
            continue;
        };
        let mut keys: Vec<&String> = by_invoice.keys().collect();
        keys.sort();
        for key in keys {
            let state = &by_invoice[key];
            if phase == Phase::Convenio && in_dataset(key) {
                continue; // the dataset entry already stands for this stop
            }
            if phase == Phase::FevValor && blocked_after(state_of(Phase::Conciliacion, key), state)
            {
                continue; // conciliar already ruled it out
            }
            let Some(category) = category_of(state.state) else {
                continue;
            };
            let (code, text) = split_reason(&state.motivo);
            items.push(ReportItem {
                factura: display
                    .get(key.as_str())
                    .map_or_else(|| state.invoice.clone(), |name| (*name).to_owned()),
                fase: phase.as_str().to_owned(),
                categoria: category,
                codigo: code,
                motivo: text,
                actualizado_en: rfc3339(state.updated_at),
            });
        }
    }
    items.sort_by(|a, b| {
        category_rank(a.categoria)
            .cmp(&category_rank(b.categoria))
            .then_with(|| phase_rank(&a.fase).cmp(&phase_rank(&b.fase)))
            .then_with(|| a.factura.cmp(&b.factura))
    });
    let mut counts = Counts::default();
    for item in &items {
        match item.categoria {
            CATEGORY_MANUAL_REVIEW => counts.revision_manual += 1,
            CATEGORY_EXCLUDED => counts.excluidas += 1,
            _ => counts.fallas += 1,
        }
    }
    Report {
        cargue: cargue.to_owned(),
        generado_en: rfc3339(now),
        incompleto: states.is_none(),
        conteos: counts,
        items,
    }
}

fn category_label(category: &str) -> &str {
    match category {
        CATEGORY_MANUAL_REVIEW => "Revisión manual",
        CATEGORY_FAILURE => "Falla",
        CATEGORY_EXCLUDED => "Excluida",
        other => other,
    }
}

/// UTF-8 BOM, `;` separator, CRLF.
fn report_csv(report: &Report) -> Vec<u8> {
    let mut csv = CsvWriter::new();
    csv.row(&[
        "Factura",
        "Fase",
        "Categoria",
        "Codigo",
        "Motivo",
        "Actualizado",
    ]);
    for item in &report.items {
        csv.row(&[
            item.factura.as_str(),
            item.fase.as_str(),
            category_label(item.categoria),
            item.codigo.as_str(),
            item.motivo.as_str(),
            item.actualizado_en.as_str(),
        ]);
    }
    csv.finish()
}

/// Loads the state of every phase for `names`; `None` when any read fails.
fn load_states(store: &ProcesoStore, names: &[String]) -> Option<PhaseStates> {
    let mut states = PhaseStates::new();
    for phase in REPORT_PHASES {
        match store.load(phase, Some(names)) {
            Ok(by_invoice) => {
                states.insert(phase, by_invoice);
            }
            Err(err) => {
                tracing::warn!(error = %err, phase = phase.as_str(), "manual review report: reading states failed");
                return None;
            }
        }
    }
    Some(states)
}

fn read_failed(err: &dyn std::fmt::Display) -> ApiError {
    tracing::warn!(error = %err, "manual review report failed");
    ApiError::internal(READ_FAILED)
}

pub fn get(state: &ApiState, req: &Request) -> ApiResult {
    let csv = match req.query_one("formato").unwrap_or_default() {
        "" | "json" => false,
        "csv" => true,
        _ => return Err(ApiError::bad_request("formato debe ser json o csv")),
    };
    let id = req.param(0);
    if !valid_cargue_id(id) {
        return Err(cargue_not_found(id));
    }
    let reports = ReportStore::new(state.pool.clone());
    let cargue = reports
        .current_cargue(id)
        .map_err(|e| read_failed(&e))?
        .ok_or_else(|| cargue_not_found(id))?;
    // A cargue with a source CSV but no prepared dataset yet has an empty report.
    let entries = reports
        .dataset_entries(cargue.seq)
        .map_err(|e| read_failed(&e))?;
    let names: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        entries
            .iter()
            .map(|e| normalize_invoice(&e.invoice))
            .filter(|n| seen.insert(n.clone()))
            .collect()
    };
    let states = if names.is_empty() {
        Some(PhaseStates::new())
    } else {
        load_states(&ProcesoStore::new(state.pool.clone()), &names)
    };
    let report = build_report(&entries, states.as_ref(), id, Utc::now());
    if !csv {
        return Ok(Response::json(&report));
    }
    let filename = format!("Informe_revision_manual_{}.csv", safe_file_name(id));
    Ok(download(&filename, report_csv(&report)))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 12, 7, minute, 0).unwrap()
    }

    fn entry(invoice: &str, state: &str, code: &str, reason: &str) -> DatasetEntry {
        DatasetEntry {
            invoice: invoice.into(),
            state: state.into(),
            code: code.into(),
            reason: reason.into(),
        }
    }

    fn proceso(
        invoice: &str,
        phase: Phase,
        state: PhaseState,
        motivo: &str,
        minute: u32,
    ) -> ProcesoState {
        ProcesoState {
            invoice: invoice.into(),
            phase,
            state,
            motivo: motivo.into(),
            intentos: 1,
            version: 0,
            cargue_id: None,
            claimed_by: None,
            claimed_at: None,
            next_attempt_at: None,
            updated_at: at(minute),
        }
    }

    fn states(rows: Vec<ProcesoState>) -> PhaseStates {
        let mut out = PhaseStates::new();
        for row in rows {
            out.entry(row.phase)
                .or_default()
                .insert(normalize_invoice(&row.invoice), row);
        }
        out
    }

    #[test]
    fn reasons_split_a_leading_code() {
        let cases = [
            ("SIN_XML: falta", ("SIN_XML", "falta")),
            ("  A1:\n  texto largo  ", ("A1", "texto largo")),
            ("MONTO_MAXIMO_SUPERADO", ("", "MONTO_MAXIMO_SUPERADO")),
            ("sin codigo: aqui no", ("", "sin codigo: aqui no")),
            ("CODE:sinespacio", ("", "CODE:sinespacio")),
            ("1CODE: x", ("", "1CODE: x")),
            ("", ("", "")),
        ];
        for (reason, (code, text)) in cases {
            assert_eq!(
                split_reason(reason),
                (code.to_owned(), text.to_owned()),
                "{reason:?}"
            );
        }
    }

    #[test]
    fn cargue_ids_are_one_path_component() {
        assert!(valid_cargue_id("Convenio_21102026_21102026"));
        assert!(valid_cargue_id("a..b"));
        for bad in ["", "..", "a/b", "a\\b"] {
            assert!(!valid_cargue_id(bad), "{bad:?}");
        }
        assert_eq!(
            cargue_not_found("x").message,
            "dashboard: cargue not found: \"x\""
        );
        assert_eq!(
            cargue_not_found("..").message,
            "dashboard: cargue not found: invalid id \"..\""
        );
    }

    #[test]
    fn the_report_mirrors_the_go_golden_shape() {
        let entries = [
            entry("FVS1", "PREPARADA", "", ""),
            entry("FVS9", "REVISION_MANUAL", "SIN_SSC", " sin ssc "),
            entry("FVS8", "EXCLUIDA", "", "MONTO_MAXIMO_SUPERADO"),
            entry("fvs-3", "PREPARADA", "", ""),
            entry("FVS2", "PREPARADA", "", ""),
            entry("FVS6", "PREPARADA", "", ""),
        ];
        let rows = states(vec![
            proceso(
                "FVS6",
                Phase::Derechos,
                PhaseState::RevisionManual,
                "SIN_DERECHOS: no vigente",
                13,
            ),
            proceso(
                "FVS3",
                Phase::TheFactory,
                PhaseState::RevisionManual,
                "SIN_XML: falta",
                11,
            ),
            proceso(
                "FVS2",
                Phase::Rips,
                PhaseState::Failed,
                "DESCARGA: timeout",
                12,
            ),
            proceso(
                "FVS3",
                Phase::FevValor,
                PhaseState::RevisionManual,
                "VALOR_DIFERENTE: x",
                10,
            ),
            proceso(
                "FVS3",
                Phase::Conciliacion,
                PhaseState::Excluded,
                "BLOQUEADA_TARIFA: precio del XML supera la nota tecnica",
                20,
            ),
            proceso("FVS1", Phase::Derechos, PhaseState::Done, "", 5),
            proceso("FVS1", Phase::Cuv, PhaseState::EnProgreso, "", 6),
        ]);
        let report = build_report(&entries, Some(&rows), "CargueA", at(30));
        let seen: Vec<(&str, &str, &str, &str, &str)> = report
            .items
            .iter()
            .map(|i| {
                (
                    i.factura.as_str(),
                    i.fase.as_str(),
                    i.categoria,
                    i.codigo.as_str(),
                    i.motivo.as_str(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("FVS9", "convenio", "revision_manual", "SIN_SSC", "sin ssc"),
                (
                    "FVS6",
                    "derechos",
                    "revision_manual",
                    "SIN_DERECHOS",
                    "no vigente"
                ),
                ("fvs-3", "thefactory", "revision_manual", "SIN_XML", "falta"),
                ("FVS2", "rips", "falla", "DESCARGA", "timeout"),
                ("FVS8", "convenio", "excluida", "", "MONTO_MAXIMO_SUPERADO"),
                (
                    "fvs-3",
                    "conciliacion",
                    "excluida",
                    "BLOQUEADA_TARIFA",
                    "precio del XML supera la nota tecnica"
                ),
            ]
        );
        assert_eq!(
            report.conteos,
            Counts {
                revision_manual: 3,
                excluidas: 2,
                fallas: 1
            }
        );
        assert!(!report.incompleto);
        assert_eq!(report.items[0].actualizado_en, "");
        assert_eq!(report.items[1].actualizado_en, "2026-09-12T07:13:00Z");
        assert_eq!(report.generado_en, "2026-09-12T07:30:00Z");
        assert_eq!(report.cargue, "CargueA");
    }

    #[test]
    fn a_fev_valor_stop_survives_unless_conciliar_blocked_it_afterwards() {
        let entries = [entry("FVS1", "PREPARADA", "", "")];
        let fev = proceso(
            "FVS1",
            Phase::FevValor,
            PhaseState::RevisionManual,
            "VALOR_DIFERENTE: x",
            10,
        );
        let blocked = |code: &str, minute: u32| {
            states(vec![
                fev.clone(),
                proceso(
                    "FVS1",
                    Phase::Conciliacion,
                    PhaseState::Excluded,
                    &format!("{code}: y"),
                    minute,
                ),
            ])
        };
        let phases = |report: &Report| -> Vec<String> {
            report.items.iter().map(|i| i.fase.clone()).collect()
        };
        // Blocked at the same instant or later: only the conciliacion stop stays.
        for minute in [10, 11] {
            let rows = blocked("BLOQUEADA_MONTO", minute);
            let report = build_report(&entries, Some(&rows), "C", at(0));
            assert_eq!(phases(&report), ["conciliacion"], "minute {minute}");
        }
        // Blocked earlier than the check, or not a permanent block: both stay.
        for (code, minute) in [("BLOQUEADA_MONTO", 9), ("OTRO", 11)] {
            let rows = blocked(code, minute);
            let report = build_report(&entries, Some(&rows), "C", at(0));
            assert_eq!(
                phases(&report),
                ["fev_valor", "conciliacion"],
                "{code} {minute}"
            );
        }
    }

    #[test]
    fn a_convenio_stop_fixed_later_is_dropped_and_a_pending_one_carries_its_time() {
        let entries = [
            entry("FVS1", "REVISION_MANUAL", "SIN_SSC", "x"),
            entry("FVS2", "EXCLUIDA", "", "MONTO: y"),
        ];
        let rows = states(vec![
            proceso("FVS1", Phase::Convenio, PhaseState::Done, "", 5),
            proceso("FVS2", Phase::Convenio, PhaseState::Excluded, "MONTO: y", 6),
        ]);
        let report = build_report(&entries, Some(&rows), "C", at(0));
        assert_eq!(report.items.len(), 1);
        let item = &report.items[0];
        assert_eq!(
            (
                item.factura.as_str(),
                item.codigo.as_str(),
                item.motivo.as_str(),
                item.actualizado_en.as_str()
            ),
            ("FVS2", "MONTO", "y", "2026-09-12T07:06:00Z")
        );
    }

    #[test]
    fn unreadable_states_leave_the_dataset_entries_flagged_incomplete() {
        let entries = [entry("FVS9", "REVISION_MANUAL", "SIN_SSC", "sin ssc")];
        let report = build_report(&entries, None, "C", at(0));
        assert!(report.incompleto);
        assert_eq!(report.items.len(), 1);
        let empty = build_report(&[], Some(&PhaseStates::new()), "C", at(0));
        assert!(!empty.incompleto && empty.items.is_empty());
    }

    #[test]
    fn the_csv_labels_categories_and_neutralizes_formulas() {
        let entries = [entry("FVS9", "REVISION_MANUAL", "SIN_SSC", "=cmd;x")];
        let report = build_report(&entries, Some(&PhaseStates::new()), "C", at(0));
        let csv = String::from_utf8(report_csv(&report)).expect("utf8");
        assert_eq!(
            csv,
            "\u{feff}Factura;Fase;Categoria;Codigo;Motivo;Actualizado\r\n\
             FVS9;convenio;Revisión manual;SIN_SSC;\"'=cmd;x\";\r\n"
        );
    }
}
