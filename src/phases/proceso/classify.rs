//! Turns what one per-invoice phase call returned into the invoice's
//! [`Verdict`] (Go `classifyOutcome`).
//!
//! * fev: `DONE` is OK; `REVISION_MANUAL` with a `VALOR_DIFERENTE ...` motivo
//!   routes the invoice to `conciliar`; `FAILED` with `XML_SIN_VALOR ...` is
//!   manual review.
//! * conciliar: an [`Incomplete`] error with technical failures is FAILED
//!   (resumable when only the Medicar portal is down); with only manual items
//!   it drops the invoice for manual review.
//! * derechos: [`DerechosError::AuthenticationStopped`] is fatal for the whole
//!   run, whatever `--continue-on-error` says.
//! * cuv: a [`MissingRipsJsonError`] for the same invoice fails only that
//!   invoice.
//! * a database error that outlived the store's retries fails only that
//!   invoice (it is retryable and says nothing about the others).
//! * every other phase: the stored outcome in the report (or, when the phase
//!   reported none, the state read back from `proceso_estado`) decides; an
//!   invoice another machine owns is `SKIPPED`, not a failure.

use anyhow::Error;

use crate::db::DbError;
use crate::domain::PhaseState;
use crate::phases::common::PhaseReport;
use crate::phases::conciliar::plan::Incomplete;
use crate::phases::cuv::MissingRipsJsonError;
use crate::phases::derechos::DerechosError;

/// Skip reason of an invoice another machine has claimed.
pub const OTHER_MACHINE: &str = "EN_PROCESO_OTRA_MAQUINA";

/// Reason shown for an invoice whose database connection stayed lost.
pub const STORE_UNAVAILABLE: &str = "conexión con la base de datos perdida: reintentos agotados";

/// Reason of a derechos authentication stop.
pub const DERECHOS_AUTH_STOP: &str = "validador de derechos: credenciales o token rechazados aun despues de renovarlo; se detiene la corrida";

/// The outcome of an invoice in a phase. `Ok` advances it; `Manual` and
/// `Excluded` drop it (a person or a business rule owns it); `Failed` drops
/// it too; `Skipped` leaves it to another machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Ok,
    Skipped,
    Excluded,
    Manual,
    Failed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::Skipped => "SKIPPED",
            Status::Excluded => "EXCLUDED",
            Status::Manual => "MANUAL_REVIEW",
            Status::Failed => "FAILED",
        }
    }
}

/// What a phase reported for one invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub status: Status,
    /// Successor phase other than the default (fev -> conciliar).
    pub route: Option<&'static str>,
    /// Human detail for the event.
    pub reason: String,
    /// A systemic failure that aborts the run (unless `--continue-on-error`).
    pub error: Option<String>,
    /// The error aborts the run even with `--continue-on-error`.
    pub fatal: bool,
    /// The error does not abort (a transient outage): the invoice stays FAILED
    /// for the next run but the error is still the run's error.
    pub resumable: bool,
    /// A transient database conflict was seen: the phase's concurrency halves.
    pub conflict: bool,
}

impl Verdict {
    pub fn new(status: Status, reason: &str) -> Self {
        Self {
            status,
            route: None,
            reason: reason.to_owned(),
            error: None,
            fatal: false,
            resumable: false,
            conflict: false,
        }
    }

    pub fn ok() -> Self {
        Self::new(Status::Ok, "")
    }

    pub fn failed_with(error: &Error) -> Self {
        Self {
            error: Some(format!("{error:#}")),
            ..Self::new(Status::Failed, "")
        }
    }
}

/// True when the chain holds a database error that a retry could cure but
/// that survived the store's own retries.
pub fn is_transient_store_error(err: &Error) -> bool {
    err.chain()
        .filter_map(|cause| cause.downcast_ref::<DbError>())
        .any(DbError::is_transient)
}

/// Classifies the result of `phase` for `invoice`. `stored` reads the state
/// the phase recorded in `proceso_estado` (only used when the report names
/// the invoice neither as stored nor as skipped).
pub fn classify(
    phase: &str,
    invoice: &str,
    result: Result<PhaseReport, Error>,
    stored: &dyn Fn() -> Option<PhaseState>,
) -> Verdict {
    match result {
        Err(err) => classify_error(phase, invoice, &err),
        Ok(report) => classify_report(phase, invoice, &report, stored),
    }
}

fn classify_error(phase: &str, invoice: &str, err: &Error) -> Verdict {
    if phase == "cuv"
        && let Some(missing) = err.downcast_ref::<MissingRipsJsonError>()
        && missing.invoice == invoice
    {
        return Verdict::new(Status::Failed, &missing.to_string());
    }
    if phase == "derechos"
        && let Some(DerechosError::AuthenticationStopped { .. }) = err.downcast_ref()
    {
        return Verdict {
            error: Some(format!("{err:#}")),
            fatal: true,
            ..Verdict::new(Status::Failed, DERECHOS_AUTH_STOP)
        };
    }
    if is_transient_store_error(err) {
        return Verdict {
            conflict: true,
            ..Verdict::new(Status::Failed, STORE_UNAVAILABLE)
        };
    }
    if phase == "conciliar"
        && let Some(incomplete) = err.downcast_ref::<Incomplete>()
    {
        if incomplete.failed > 0 {
            return Verdict {
                error: Some(format!("{err:#}")),
                resumable: incomplete.only_medicar_unavailable(),
                ..Verdict::new(Status::Failed, "")
            };
        }
        return Verdict::new(Status::Manual, &incomplete.to_string());
    }
    Verdict::failed_with(err)
}

fn classify_report(
    phase: &str,
    invoice: &str,
    report: &PhaseReport,
    stored: &dyn Fn() -> Option<PhaseState>,
) -> Verdict {
    if let Some(done) = report.outcomes.iter().find(|o| o.invoice == invoice) {
        let (state, motivo) = (done.outcome.state, done.outcome.motivo.as_str());
        return if phase == "fev" {
            fev_verdict(state, motivo)
        } else {
            state_verdict(state)
        };
    }
    if let Some((_, reason)) = report.skipped.iter().find(|(inv, _)| inv == invoice) {
        return match reason.as_str() {
            OTHER_MACHINE => Verdict::new(Status::Skipped, OTHER_MACHINE),
            "INTERRUMPIDO" => Verdict::new(Status::Skipped, "INTERRUMPIDO"),
            // The phase refused the invoice (for example a convenio that did
            // not prepare it): it must not advance.
            other => Verdict::new(Status::Manual, other),
        };
    }
    match stored() {
        // Still claimed by someone and the phase did not say so: not ours.
        Some(PhaseState::EnProgreso) => Verdict::new(Status::Skipped, OTHER_MACHINE),
        Some(state) => state_verdict(state),
        None => Verdict::ok(),
    }
}

fn state_verdict(state: PhaseState) -> Verdict {
    match state {
        PhaseState::Done | PhaseState::EnProgreso => Verdict::ok(),
        PhaseState::Excluded => Verdict::new(Status::Excluded, state.as_str()),
        PhaseState::RevisionManual => Verdict::new(Status::Manual, state.as_str()),
        PhaseState::Failed => Verdict::new(Status::Failed, state.as_str()),
    }
}

fn fev_verdict(state: PhaseState, motivo: &str) -> Verdict {
    match state {
        PhaseState::RevisionManual if motivo.starts_with("VALOR_DIFERENTE") => Verdict {
            route: Some("conciliar"),
            ..Verdict::new(Status::Ok, "VALOR_DIFERENTE")
        },
        PhaseState::Failed if motivo.starts_with("XML_SIN_VALOR") => {
            Verdict::new(Status::Manual, "XML_SIN_VALOR")
        }
        other => state_verdict(other),
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::*;
    use crate::domain::Outcome;
    use crate::phases::conciliar::plan::Detail;

    fn report(invoice: &str, outcome: Outcome) -> PhaseReport {
        let mut report = PhaseReport::default();
        report.push(invoice, outcome);
        report
    }

    fn none() -> Option<PhaseState> {
        None
    }

    #[test]
    fn fev_routes_value_differences_to_conciliar_and_missing_values_to_manual() {
        let diff = classify(
            "fev",
            "A1",
            Ok(report(
                "A1",
                Outcome::manual_review("VALOR_DIFERENTE XML=1 convenio=2"),
            )),
            &none,
        );
        assert_eq!((diff.status, diff.route), (Status::Ok, Some("conciliar")));
        assert_eq!(diff.reason, "VALOR_DIFERENTE");
        let nil = classify(
            "fev",
            "A1",
            Ok(report(
                "A1",
                Outcome::failed("XML_SIN_VALOR (no se pudo extraer el valor total del XML)"),
            )),
            &none,
        );
        assert_eq!(
            (nil.status, nil.reason.as_str()),
            (Status::Manual, "XML_SIN_VALOR")
        );
        let ok = classify("fev", "A1", Ok(report("A1", Outcome::done())), &none);
        assert_eq!((ok.status, ok.route), (Status::Ok, None));
        let other = classify(
            "fev",
            "A1",
            Ok(report("A1", Outcome::failed("boom"))),
            &none,
        );
        assert_eq!(other.status, Status::Failed);
        assert!(other.error.is_none(), "a recorded failure does not abort");
    }

    #[test]
    fn recorded_states_drop_the_invoice_without_aborting() {
        for (outcome, want) in [
            (Outcome::excluded("x"), Status::Excluded),
            (Outcome::manual_review("x"), Status::Manual),
            (Outcome::failed("x"), Status::Failed),
            (Outcome::done(), Status::Ok),
        ] {
            let v = classify("rips", "A1", Ok(report("A1", outcome)), &none);
            assert_eq!(v.status, want);
            assert!(v.error.is_none());
        }
    }

    #[test]
    fn a_report_without_the_invoice_reads_the_state_back() {
        let stored = || Some(PhaseState::RevisionManual);
        let v = classify("cuv", "A1", Ok(PhaseReport::default()), &stored);
        assert_eq!(
            (v.status, v.reason.as_str()),
            (Status::Manual, "REVISION_MANUAL")
        );
        let v = classify("cuv", "A1", Ok(PhaseReport::default()), &none);
        assert_eq!(v.status, Status::Ok);
    }

    #[test]
    fn an_invoice_owned_by_another_machine_is_skipped_not_failed() {
        let mut skipped = PhaseReport::default();
        skipped.skip("A1", OTHER_MACHINE);
        let v = classify("derechos", "A1", Ok(skipped), &none);
        assert_eq!(
            (v.status, v.reason.as_str()),
            (Status::Skipped, OTHER_MACHINE)
        );
        let mut refused = PhaseReport::default();
        refused.skip("A1", "CONVENIO_NO_OK");
        let v = classify("rips", "A1", Ok(refused), &none);
        assert_eq!(
            (v.status, v.reason.as_str()),
            (Status::Manual, "CONVENIO_NO_OK")
        );
    }

    #[test]
    fn a_derechos_authentication_stop_is_fatal() {
        let err = anyhow::Error::new(DerechosError::AuthenticationStopped {
            untouched: 1,
            detail: "401".into(),
        });
        let v = classify("derechos", "A1", Err(err), &none);
        assert!(v.fatal && v.error.is_some());
        assert_eq!(v.status, Status::Failed);
        let other = anyhow::Error::new(DerechosError::Interrupted);
        let v = classify("derechos", "A1", Err(other), &none);
        assert!(!v.fatal && v.error.is_some());
    }

    #[test]
    fn a_missing_rips_json_fails_only_its_invoice() {
        let err = anyhow::Error::new(MissingRipsJsonError {
            invoice: "A1".into(),
        });
        let v = classify("cuv", "A1", Err(err), &none);
        assert_eq!(v.status, Status::Failed);
        assert!(v.error.is_none() && v.reason.starts_with("RIPS_JSON_FALTA"));
        let foreign = anyhow::Error::new(MissingRipsJsonError {
            invoice: "B2".into(),
        });
        assert!(classify("cuv", "A1", Err(foreign), &none).error.is_some());
        assert!(
            classify("cuv", "A1", Err(anyhow!("io")), &none)
                .error
                .is_some()
        );
    }

    fn incomplete(manual: usize, failed: usize, unavailable: bool) -> Incomplete {
        Incomplete {
            manual_review: manual,
            failed,
            details: vec![Detail {
                invoice: "A1".into(),
                code: "MEDICAR".into(),
                reason: "down".into(),
                failed: failed > 0,
                unavailable,
            }],
        }
    }

    #[test]
    fn conciliar_distinguishes_manual_items_failures_and_a_medicar_outage() {
        let manual = classify(
            "conciliar",
            "A1",
            Err(incomplete(1, 0, false).into()),
            &none,
        );
        assert_eq!(manual.status, Status::Manual);
        assert!(manual.error.is_none());
        let failed = classify(
            "conciliar",
            "A1",
            Err(incomplete(0, 1, false).into()),
            &none,
        );
        assert!(failed.error.is_some() && !failed.resumable);
        let down = classify("conciliar", "A1", Err(incomplete(0, 1, true).into()), &none);
        assert!(down.error.is_some() && down.resumable);
        assert_eq!(down.status, Status::Failed);
    }

    #[test]
    fn a_lost_database_fails_the_invoice_but_not_the_run() {
        let lost = anyhow::Error::new(DbError::Other("x".into()));
        assert!(classify("rips", "A1", Err(lost), &none).error.is_some());
        let pool = DbError::Query(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::SerializationFailure,
            Box::new("conflict".to_owned()),
        ));
        let v = classify("rips", "A1", Err(anyhow::Error::new(pool)), &none);
        assert_eq!(v.status, Status::Failed);
        assert!(v.error.is_none() && v.conflict);
        assert_eq!(v.reason, STORE_UNAVAILABLE);
    }
}
