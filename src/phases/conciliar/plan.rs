//! Per-invoice decisions of `conciliar` (CONC-04, CONC-06, CONC-08, CONC-14).
//!
//! [`decide`] is the first half of the Go command loop: from the invoice, its
//! XML and the catalog it chooses to skip it, to record a final outcome
//! (`ERROR_XML`, `DIFERENCIA_PERSISTE`, `BLOQUEADA_*`) or to leave it
//! [`Pending`] for the Medicar date. [`finish_pending`] is the second half:
//! once Medicar answered it rebuilds the lines or maps the failure to a state.
//! Both are pure; the console lines are returned, not printed.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::catalog::Catalog;
use super::medicar::MedicarError;
use super::portal_html::Formula;
use super::rules::{Analysis, Estado, RebuildInput, analyze, cents, money, rebuild};
use crate::domain::dataset::Invoice;
use crate::domain::{Outcome, PhaseState};
use crate::fevxml::{invoice_lines, numero_convenio, read_total};

/// Result counters, printed in the summary line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub conciliadas: usize,
    pub bloqueadas: usize,
    pub diferencia: usize,
    pub revision_manual: usize,
    pub fallidas: usize,
    pub omitidas: usize,
}

/// Which counter an invoice adds to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountKey {
    Conciliadas,
    Bloqueadas,
    Diferencia,
    RevisionManual,
    Fallidas,
    Omitidas,
}

impl Counts {
    pub fn add(&mut self, key: CountKey) {
        match key {
            CountKey::Conciliadas => self.conciliadas += 1,
            CountKey::Bloqueadas => self.bloqueadas += 1,
            CountKey::Diferencia => self.diferencia += 1,
            CountKey::RevisionManual => self.revision_manual += 1,
            CountKey::Fallidas => self.fallidas += 1,
            CountKey::Omitidas => self.omitidas += 1,
        }
    }

    /// `conciliar: conciliadas=… bloqueadas=… diferencia=… revision_manual=… fallidas=… omitidas=…`
    pub fn summary_line(&self) -> String {
        format!(
            "conciliar: conciliadas={} bloqueadas={} diferencia={} revision_manual={} fallidas={} omitidas={}",
            self.conciliadas,
            self.bloqueadas,
            self.diferencia,
            self.revision_manual,
            self.fallidas,
            self.omitidas
        )
    }

    /// The error that makes the command exit non-zero: any invoice blocked,
    /// still different, in manual review or failed.
    pub fn incomplete(&self, details: Vec<Detail>) -> Option<Incomplete> {
        let manual = self.bloqueadas + self.diferencia + self.revision_manual;
        (manual + self.fallidas > 0).then_some(Incomplete {
            manual_review: manual,
            failed: self.fallidas,
            details,
        })
    }
}

/// One invoice `conciliar` could not reconcile: the outcome code
/// (`BLOQUEADA_TARIFA`, `MEDICAR`, ...), the reason and whether it is a
/// technical failure to retry or a case for manual review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    pub invoice: String,
    pub code: String,
    pub reason: String,
    pub failed: bool,
    /// A failed one caused only by the Medicar portal being down.
    pub unavailable: bool,
}

const MAX_LISTED: usize = 5;
const MAX_REASON: usize = 240;
const REASON_HINT: &str = "Motivo completo en la página «Revisión manual» o en el log.";

/// Invoices `conciliar` could not reconcile. `proceso run` continues its chain
/// on manual review but aborts on a technical failure, except when only the
/// portal is down ([`Incomplete::only_medicar_unavailable`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incomplete {
    pub manual_review: usize,
    pub failed: usize,
    pub details: Vec<Detail>,
}

impl Incomplete {
    /// Every unreconciled invoice failed because the Medicar portal is down:
    /// no manual review and no other technical failure.
    pub fn only_medicar_unavailable(&self) -> bool {
        if self.manual_review > 0 || self.failed == 0 {
            return false;
        }
        let down = self
            .details
            .iter()
            .filter(|d| d.failed && d.unavailable)
            .count();
        down == self.failed
    }
}

impl std::fmt::Display for Incomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} factura(s) sin conciliar",
            self.manual_review + self.failed
        )?;
        if self.details.is_empty() {
            return Ok(());
        }
        let mut parts = Vec::new();
        for (i, d) in self.details.iter().enumerate() {
            if i == MAX_LISTED {
                parts.push(format!("y {} más", self.details.len() - MAX_LISTED));
                break;
            }
            let kind = if d.failed {
                "fallo técnico, reintentar"
            } else {
                "revisión manual"
            };
            parts.push(format!(
                "{} → {} ({kind}): {}",
                d.invoice,
                d.code,
                one_line(&d.reason, MAX_REASON)
            ));
        }
        write!(f, ": {}. {REASON_HINT}", parts.join("; "))
    }
}

impl std::error::Error for Incomplete {}

/// Collapses all whitespace into single spaces and cuts the result to `max`
/// characters, marking the cut with an ellipsis.
pub fn one_line(s: &str, max: usize) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() > max {
        let cut: String = joined.chars().take(max).collect();
        return format!("{cut}…");
    }
    joined
}

/// The conciliación state an invoice already has (loaded only when
/// `--invoices` is not given).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnState {
    pub state: PhaseState,
    pub motivo: String,
}

/// An invoice and what is known about it.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    pub invoice: &'a Invoice,
    /// The invoice XML (first `.xml` entry of its ZIP, else the loose file).
    pub xml: Option<&'a [u8]>,
    /// `None` when the invoices were requested explicitly.
    pub own: Option<&'a OwnState>,
}

/// The catalog and cap shared by all the invoices of a run.
#[derive(Debug, Clone, Copy)]
pub struct Rules<'a> {
    pub catalog: &'a Catalog,
    /// Per-invoice cap in cents; `None` = unchecked.
    pub max_cents: Option<i64>,
}

/// A final outcome decided without Medicar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub outcome: Outcome,
    pub key: CountKey,
    pub line: String,
    pub detail: Detail,
}

/// A `CONCILIADA` invoice waiting for its formula date.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub analysis: Analysis,
    /// XML total as printed (`valor_xml`).
    pub xml_text: String,
    /// Convenio total as printed, or `NO_DISPONIBLE`.
    pub prev_text: String,
}

/// What to do with an invoice.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Not a candidate: counted as `omitidas`, not recorded.
    Skip { line: String },
    /// Final outcome: recorded in `proceso_estado` phase `conciliacion`.
    Record(Recorded),
    /// Needs the Medicar formula date.
    Pending(Box<Pending>),
}

/// The sum of every line's "vlr. total" (`totalConvenioDtm`); `None` when a
/// value cannot be read or there are no lines.
pub fn convenio_total(invoice: &Invoice) -> Option<Decimal> {
    let mut total = Decimal::ZERO;
    for l in &invoice.lines {
        total += numero_convenio(&l.total_value)?;
    }
    (!invoice.lines.is_empty()).then_some(total)
}

fn skip(line: String) -> Decision {
    Decision::Skip { line }
}

fn record(
    inv: &str,
    state: PhaseState,
    key: CountKey,
    code: &str,
    reason: &str,
    detail: &str,
    line: String,
) -> Decision {
    let outcome = Outcome::new(state, reason);
    Decision::Record(Recorded {
        outcome,
        key,
        line,
        detail: Detail {
            invoice: inv.to_owned(),
            code: code.to_owned(),
            reason: detail.to_owned(),
            failed: false,
            unavailable: false,
        },
    })
}

/// Recomputes the XML-versus-convenio comparison and decides (CONC-04..06).
pub fn decide(c: &Candidate<'_>, rules: &Rules<'_>) -> Decision {
    let inv = c.invoice.invoice.as_str();
    if let Some(own) = c
        .own
        .filter(|o| matches!(o.state, PhaseState::Excluded | PhaseState::RevisionManual))
    {
        return skip(format!(
            "  skip {inv} CONCILIACION {}: {} (reprocess with --invoices)",
            own.state, own.motivo
        ));
    }
    let xml_total = c.xml.and_then(read_total);
    let (Some(content), Some(xml_total)) = (c.xml, xml_total) else {
        return skip(format!(
            "  skip {inv} XML_SIN_VALOR (no se pudo extraer el valor total del XML; ver fev verificar)"
        ));
    };
    let conv = convenio_total(c.invoice);
    let prev_text = conv.map_or_else(|| "NO_DISPONIBLE".to_owned(), |d| d.to_string());
    let xml_cents = cents(xml_total);
    if conv.is_some_and(|d| cents(d) == xml_cents) {
        return skip(format!(
            "  skip {inv} SIN_DIFERENCIA XML={xml_total} convenio={prev_text}"
        ));
    }
    let analysis = invoice_lines(content)
        .map_err(|e| e.to_string())
        .and_then(|lines| {
            analyze(c.invoice, xml_cents, &lines, rules.catalog, rules.max_cents)
                .map_err(|e| e.to_string())
        });
    let analysis = match analysis {
        Ok(a) => a,
        Err(err) => {
            let reason = format!("ERROR_XML: {err}");
            let line = format!("  {inv} {reason}");
            return record(
                inv,
                PhaseState::RevisionManual,
                CountKey::RevisionManual,
                "ERROR_XML",
                &reason,
                &err,
                line,
            );
        }
    };
    let added = analysis.lines.iter().filter(|l| l.base.is_none()).count();
    let summary = format!(
        "XML={xml_total} convenio_anterior={prev_text} convenio_conciliado={} lineas_xml={} agregadas={added} no_confirmadas={}",
        money(analysis.total),
        analysis.lines.len(),
        analysis.unconfirmed.len()
    );
    let state = analysis.state;
    let name = state.as_str();
    let line = format!("  {inv} {name} {summary}: {}", analysis.detail);
    let reason = format!("{name}: {} {summary}", analysis.detail);
    match state {
        Estado::Conciliada => Decision::Pending(Box::new(Pending {
            analysis,
            xml_text: xml_total.to_string(),
            prev_text,
        })),
        Estado::Diferencia => {
            let detail = analysis.detail.clone();
            record(
                inv,
                PhaseState::RevisionManual,
                CountKey::Diferencia,
                name,
                &reason,
                &detail,
                line,
            )
        }
        Estado::Tarifa | Estado::Monto => {
            let detail = analysis.detail.clone();
            record(
                inv,
                PhaseState::Excluded,
                CountKey::Bloqueadas,
                name,
                &reason,
                &detail,
                line,
            )
        }
    }
}

/// An invoice whose `<F>.txt` already holds a CUV for the previous values:
/// manual review, Medicar is not queried (CONC-08).
pub fn cuv_existente(invoice: &str) -> Recorded {
    let detail = format!(
        "{invoice}.txt ya tiene CUV con los valores anteriores; requiere revision (nota/anulacion) antes de reconciliar"
    );
    let reason = format!("CUV_EXISTENTE: {detail}");
    Recorded {
        outcome: Outcome::manual_review(&reason),
        key: CountKey::RevisionManual,
        line: format!("  {invoice} {reason}"),
        detail: Detail {
            invoice: invoice.to_owned(),
            code: "CUV_EXISTENTE".into(),
            reason: detail,
            failed: false,
            unavailable: false,
        },
    }
}

/// The outcome of a pending invoice once Medicar answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    /// Result for phase `conciliacion`.
    pub conciliacion: Outcome,
    /// Result for phase `fev_valor` (`DONE` when reconciled, the Medicar
    /// failure otherwise).
    pub fev_valor: Outcome,
    /// The rebuilt invoice, when Medicar gave the date.
    pub rebuilt: Option<Invoice>,
    pub key: CountKey,
    pub line: String,
    /// Set for a failure.
    pub detail: Option<Detail>,
}

/// Maps the Medicar answer to a state (CONC-07, CONC-08) and rebuilds the
/// lines on success. `now` stamps `conciliado_en`.
pub fn finish_pending(
    invoice: &Invoice,
    pending: &Pending,
    formula: Result<Formula, MedicarError>,
    now: DateTime<Utc>,
) -> Finished {
    let inv = invoice.invoice.as_str();
    match formula {
        Err(err) => medicar_failure(inv, &err),
        Ok(f) => {
            let first_ssc = invoice
                .lines
                .first()
                .map(|l| l.ssc.as_str())
                .unwrap_or_default();
            let a = &pending.analysis;
            let total = money(a.total);
            let rebuilt = rebuild(
                invoice,
                a,
                &RebuildInput {
                    formula_date: &f.fecha_formula,
                    ssc: first_ssc,
                    xml_total: &pending.xml_text,
                    previous_total: &pending.prev_text,
                    now,
                },
            );
            let (state, xml, prev) = (a.state.as_str(), &pending.xml_text, &pending.prev_text);
            let reason = format!(
                "{state}: {} XML={xml} convenio_anterior={prev} convenio_conciliado={total} fecha_medicar={} SSC={first_ssc}",
                a.detail, f.fecha_formula
            );
            Finished {
                conciliacion: Outcome::new(PhaseState::Done, &reason),
                fev_valor: Outcome::new(
                    PhaseState::Done,
                    format!("CONCILIADA por conciliar: XML={xml} convenio={total}"),
                ),
                rebuilt: Some(rebuilt),
                key: CountKey::Conciliadas,
                line: format!(
                    "  {inv} {state} XML={xml} convenio_anterior={prev} convenio_conciliado={total} fecha_medicar={}",
                    f.fecha_formula
                ),
                detail: None,
            }
        }
    }
}

fn medicar_failure(inv: &str, err: &MedicarError) -> Finished {
    let down = err.is_unavailable();
    let (state, key) = if err.is_permanent() {
        (PhaseState::RevisionManual, CountKey::RevisionManual)
    } else {
        (PhaseState::Failed, CountKey::Fallidas)
    };
    let (reason, detail) = if down {
        // The portal is down, not wrong: the invoice stays FAILED and the next
        // run retries it.
        let detail = format!("no disponible: {err}; se reintentara en la proxima ejecucion");
        (format!("MEDICAR {detail}"), detail)
    } else {
        (format!("MEDICAR: {err}"), err.to_string())
    };
    Finished {
        conciliacion: Outcome::new(state, &reason),
        fev_valor: Outcome::new(state, &reason),
        rebuilt: None,
        key,
        line: format!("  {inv} {reason}"),
        detail: Some(Detail {
            invoice: inv.to_owned(),
            code: "MEDICAR".into(),
            reason: detail,
            failed: state == PhaseState::Failed,
            unavailable: down,
        }),
    }
}
