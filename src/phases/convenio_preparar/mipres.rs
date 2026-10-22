//! MIPRES delivery check of EVENTO NO PBS invoices (port of Go
//! `convenio/mipres.go`).
//!
//! Runs last, only for invoices that passed every local rule. Per line with
//! both a prescription (`mipres`) and a delivery id (`no. formula`) the
//! deliveries of the prescription are asked once per run (failures are cached
//! too). Outcomes:
//!
//! * any matching delivery in state 2 passes the line (its id is stored);
//! * otherwise annulled (0) excludes with `MIPRES ANULADO`, then not
//!   delivered (1) with `MIPRES NO ENTREGADO`;
//! * no delivery with that id excludes with `MIPRES SIN ENTREGA: ...`;
//! * a failed query or an unknown state holds the invoice in manual review
//!   (`MIPRES_NO_VERIFICADO`);
//! * an outage trips a run-level breaker: nothing else is asked and the
//!   remaining invoices that need MIPRES are held for the next run;
//! * a systemic error (rejected credentials, interrupt) aborts the run.

use std::collections::HashMap;

use super::subplan::subplan_of;
use crate::domain::dataset::{Line, ManualReview};
use crate::util::text::normalize_key;

/// Normalized subplan whose invoices need the delivery check.
pub const EVENTO_NO_PBS: &str = "EVENTO NO PBS";

pub const REASON_MIPRES_ANULADO: &str = "MIPRES ANULADO";
pub const REASON_MIPRES_NO_ENTREGADO: &str = "MIPRES NO ENTREGADO";
/// MIPRES answered and lists no delivery with the line's No. Formula.
pub const REASON_MIPRES_SIN_ENTREGA: &str = "MIPRES SIN ENTREGA";
/// The query failed or returned an unknown state.
pub const CODE_MIPRES_NO_VERIFICADO: &str = "MIPRES_NO_VERIFICADO";

/// MIPRES delivery states (`EstEntrega`).
pub const STATE_ANULADO: i32 = 0;
pub const STATE_SIN_ENTREGA: i32 = 1;
pub const STATE_ENTREGADO: i32 = 2;

/// True when `subplan` is EVENTO NO PBS (case/accent/space tolerant).
pub fn is_evento_no_pbs(subplan: &str) -> bool {
    normalize_key(subplan) == EVENTO_NO_PBS
}

/// One delivery of a prescription; `id` is our No. Formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub id: String,
    pub state: i32,
}

/// Why a query failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MipresError {
    /// Systemic (credentials rejected, interrupt): the preparation aborts.
    #[error("MIPRES check aborted: {0}")]
    Abort(String),
    /// MIPRES could not be reached or kept failing: trips the run breaker.
    #[error("MIPRES unavailable: {0}")]
    Unavailable(String),
    /// Specific to one prescription: manual review of its invoice.
    #[error("{0}")]
    Other(String),
}

/// Lists the deliveries of a MIPRES prescription.
pub trait MipresChecker: Sync {
    fn entregas(&self, prescription: &str) -> Result<Vec<Delivery>, MipresError>;
}

/// What checking one invoice decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every checked line is delivered; the distinct delivery ids joined by
    /// `, ` (empty when no line was checked).
    Pass(String),
    /// The invoice is excluded with this reason.
    Exclude(String),
    /// The invoice is held; `unavailable` marks an outage hold.
    Hold {
        review: ManualReview,
        unavailable: bool,
    },
}

type Answer = Result<Vec<Delivery>, MipresError>;

/// Per-run state: the per-prescription cache and the breakers.
pub struct MipresRun<'a> {
    checker: &'a dyn MipresChecker,
    interrupted: &'a dyn Fn() -> bool,
    cache: HashMap<String, Answer>,
    fatal: Option<MipresError>,
    down: Option<MipresError>,
}

impl<'a> MipresRun<'a> {
    /// `interrupted` is polled before every query (Go: a canceled context).
    pub fn new(checker: &'a dyn MipresChecker, interrupted: &'a dyn Fn() -> bool) -> Self {
        Self {
            checker,
            interrupted,
            cache: HashMap::new(),
            fatal: None,
            down: None,
        }
    }

    /// The systemic error that stops the run, once one happened.
    pub fn fatal(&self) -> Option<&MipresError> {
        self.fatal.as_ref()
    }

    fn entregas(&mut self, prescription: &str) -> Answer {
        if let Some(answer) = self.cache.get(prescription) {
            return answer.clone();
        }
        if let Some(fatal) = &self.fatal {
            return Err(fatal.clone());
        }
        if let Some(down) = &self.down {
            return Err(down.clone());
        }
        if (self.interrupted)() {
            let error = MipresError::Abort("interrumpido".into());
            self.fatal = Some(error.clone());
            return Err(error);
        }
        let answer = self.checker.entregas(prescription);
        match &answer {
            Err(error @ MipresError::Abort(_)) => {
                self.fatal = Some(error.clone());
                return answer;
            }
            Err(error @ MipresError::Unavailable(_)) => {
                self.down = Some(error.clone());
                return answer;
            }
            _ => {}
        }
        self.cache.insert(prescription.to_owned(), answer.clone());
        answer
    }

    /// Verifies the lines that name both a prescription and a delivery id;
    /// the first failing line (in line order) decides.
    pub fn check_invoice(&mut self, invoice: &str, lines: &[Line]) -> Verdict {
        let mut seen: Vec<String> = Vec::new();
        for line in lines {
            let prescription = line.mipres.trim();
            let formula = line.formula_number.trim();
            if prescription.is_empty() || formula.is_empty() {
                continue;
            }
            match self.check_line(invoice, prescription, formula) {
                LineVerdict::Delivered(id) => {
                    if !seen.contains(&id) {
                        seen.push(id);
                    }
                }
                LineVerdict::Exclude(reason) => return Verdict::Exclude(reason),
                LineVerdict::Hold {
                    review,
                    unavailable,
                } => {
                    return Verdict::Hold {
                        review,
                        unavailable,
                    };
                }
            }
        }
        Verdict::Pass(seen.join(", "))
    }

    fn check_line(&mut self, invoice: &str, prescription: &str, formula: &str) -> LineVerdict {
        let hold = |reason: String, unavailable: bool| LineVerdict::Hold {
            review: ManualReview {
                invoice: invoice.to_owned(),
                code: CODE_MIPRES_NO_VERIFICADO.to_owned(),
                reason,
            },
            unavailable,
        };
        let deliveries = match self.entregas(prescription) {
            Ok(deliveries) => deliveries,
            Err(error @ MipresError::Unavailable(_)) => {
                return hold(
                    format!(
                        "MIPRES no estaba disponible; la factura queda en revision manual y se reevaluara en la proxima ejecucion (prescripcion={prescription}, No. Formula={formula}): {error}"
                    ),
                    true,
                );
            }
            Err(error) => {
                return hold(
                    format!(
                        "No se pudo consultar la entrega MIPRES (prescripcion={prescription}, No. Formula={formula}): {error}"
                    ),
                    false,
                );
            }
        };
        let matched: Vec<&Delivery> = deliveries
            .iter()
            .filter(|d| same_delivery_id(&d.id, formula))
            .collect();
        let Some(first) = matched.first() else {
            return LineVerdict::Exclude(format!(
                "{REASON_MIPRES_SIN_ENTREGA}: prescripcion={prescription}, No. Formula={formula} (MIPRES no lista una entrega con ese ID)"
            ));
        };
        if let Some(delivered) = matched.iter().find(|d| d.state == STATE_ENTREGADO) {
            return LineVerdict::Delivered(delivered.id.trim().to_owned());
        }
        if matched.iter().any(|d| d.state == STATE_ANULADO) {
            return LineVerdict::Exclude(REASON_MIPRES_ANULADO.to_owned());
        }
        if matched.iter().any(|d| d.state == STATE_SIN_ENTREGA) {
            return LineVerdict::Exclude(REASON_MIPRES_NO_ENTREGADO.to_owned());
        }
        hold(
            format!(
                "MIPRES devolvio un estado de entrega desconocido (prescripcion={prescription}, No. Formula={formula}, estado={}).",
                first.state
            ),
            false,
        )
    }
}

enum LineVerdict {
    Delivered(String),
    Exclude(String),
    Hold {
        review: ManualReview,
        unavailable: bool,
    },
}

/// Compares trimmed ids; all-digit ids compare numerically so a zero-padded
/// export (`007`) still matches the service's `7`.
pub fn same_delivery_id(a: &str, b: &str) -> bool {
    normalize_delivery_id(a) == normalize_delivery_id(b)
}

fn normalize_delivery_id(id: &str) -> String {
    let id = id.trim();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return id.to_owned();
    }
    let stripped = id.trim_start_matches('0');
    if stripped.is_empty() {
        "0".to_owned()
    } else {
        stripped.to_owned()
    }
}

/// True when some invoice the preparation would look at is EVENTO NO PBS and
/// would be prepared (in the automatic list, or named in `only`): the MIPRES
/// service will be queried at all.
pub fn needs_mipres(
    lines: &[Line],
    only: &[String],
    automatic: &super::subplan::AutomaticSubplans,
) -> bool {
    let selected = super::subplan::selected_set(only);
    let mut groups: HashMap<&str, Vec<&Line>> = HashMap::new();
    for line in lines {
        if line.invoice.is_empty() || (!selected.is_empty() && !selected.contains(&line.invoice)) {
            continue;
        }
        groups.entry(line.invoice.as_str()).or_default().push(line);
    }
    groups.iter().any(|(invoice, group)| {
        is_evento_no_pbs(&subplan_of(group.iter().copied()))
            && (selected.contains(*invoice) || automatic.allows(EVENTO_NO_PBS))
    })
}
