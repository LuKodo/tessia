//! The contract between `proceso run` and the phases, plus shared CLI flags.
//!
//! A phase's `run_phase` receives a [`PhaseRequest`], does the work for the
//! invoices of the cargue (claiming them in `proceso_estado`, running its
//! worker threads, storing outcomes) and returns a [`PhaseReport`]. The
//! standalone command (`run`) is the same function plus printing.
//!
//! Recommended shape of a phase's `run_phase`:
//!
//! 1. resolve the invoices (`req.invoices`, or all of the cargue's);
//! 2. `ctx.proceso().load(phase, ..)`, apply `domain::phase::gate` for the ones
//!    not asked for explicitly, and put the skipped ones in `report.skipped`;
//! 3. `claim_available` the rest with `ctx.claim_options(cargue_id)`;
//! 4. work with `util::par::map` (`req.workers` or the phase default), calling
//!    `save_outcome` per invoice; a `false` return means the claim was taken
//!    over: drop the result;
//! 5. return the outcomes that were stored.

use anyhow::Result;
use clap::Args;

use crate::domain::{Outcome, PhaseState, normalize_invoice};

/// What the orchestrator asks of a phase.
#[derive(Debug, Clone, Default)]
pub struct PhaseRequest {
    /// The cargue (CSV stem) being processed.
    pub cargue_id: String,
    /// Only these invoices (normalized); `None` = every invoice of the cargue.
    /// Explicitly requested invoices bypass the gate, as `--invoices` does in Go.
    pub invoices: Option<Vec<String>>,
    /// Worker threads; `None` = the phase's default.
    pub workers: Option<usize>,
}

impl PhaseRequest {
    pub fn new(cargue_id: &str) -> Self {
        Self {
            cargue_id: cargue_id.to_owned(),
            ..Self::default()
        }
    }

    /// `workers` or `default`, at least 1.
    pub fn workers_or(&self, default: usize) -> usize {
        self.workers.unwrap_or(default).max(1)
    }
}

/// The result stored for one invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceOutcome {
    pub invoice: String,
    pub outcome: Outcome,
}

/// Counters of a phase run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub done: usize,
    pub failed: usize,
    pub excluded: usize,
    pub manual_review: usize,
    /// Skipped by the gate or because another machine owns them.
    pub skipped: usize,
}

/// What a phase did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhaseReport {
    /// Outcomes this run stored, in processing order.
    pub outcomes: Vec<InvoiceOutcome>,
    /// `(invoice, reason)` the run did not process (gate reason such as
    /// `RIPS_NO_OK`, or `EN_PROCESO_OTRA_MAQUINA`).
    pub skipped: Vec<(String, String)>,
}

impl PhaseReport {
    pub fn push(&mut self, invoice: &str, outcome: Outcome) {
        self.outcomes.push(InvoiceOutcome {
            invoice: invoice.to_owned(),
            outcome,
        });
    }

    pub fn skip(&mut self, invoice: &str, reason: &str) {
        self.skipped.push((invoice.to_owned(), reason.to_owned()));
    }

    pub fn counters(&self) -> Counters {
        let mut counters = Counters {
            skipped: self.skipped.len(),
            ..Counters::default()
        };
        for stored in &self.outcomes {
            match stored.outcome.state {
                PhaseState::Done => counters.done += 1,
                PhaseState::Failed => counters.failed += 1,
                PhaseState::Excluded => counters.excluded += 1,
                PhaseState::RevisionManual => counters.manual_review += 1,
                PhaseState::EnProgreso => {}
            }
        }
        counters
    }

    /// One Spanish summary line for the end of a command.
    pub fn summary(&self) -> String {
        let c = self.counters();
        format!(
            "{} procesadas: {} listas, {} fallidas, {} excluidas, {} en revisión manual, {} omitidas",
            self.outcomes.len(),
            c.done,
            c.failed,
            c.excluded,
            c.manual_review,
            c.skipped
        )
    }

    /// True when any invoice failed (the command should exit non-zero only
    /// when its Go counterpart does; most phases keep FAILED as data).
    pub fn has_failures(&self) -> bool {
        self.counters().failed > 0
    }
}

/// Flags shared by the phase commands (flatten them into a phase's `Args`
/// with `#[command(flatten)]`).
#[derive(Debug, Clone, Args, Default)]
pub struct CommonArgs {
    /// Cargue a procesar (el nombre del CSV sin extensión, p. ej. Convenio_21102026_21102026)
    #[arg(long, value_name = "ID")]
    pub cargue: Option<String>,
    /// Procesar solo estas facturas (separadas por coma); se saltan las compuertas de fase
    #[arg(long, value_name = "FACTURAS", value_delimiter = ',')]
    pub invoices: Vec<String>,
    /// Hilos de trabajo (por defecto el de cada fase)
    #[arg(long, value_name = "N")]
    pub workers: Option<usize>,
}

impl CommonArgs {
    /// The cargue id, or an error naming the flag.
    pub fn require_cargue(&self) -> Result<&str> {
        self.cargue
            .as_deref()
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("falta --cargue <ID> (el nombre del CSV sin extensión)"))
    }

    /// Builds the request; invoices are normalized and deduplicated.
    pub fn to_request(&self) -> Result<PhaseRequest> {
        let invoices = crate::domain::invoice::normalize_all(
            self.invoices.iter().map(|i| normalize_invoice(i)),
        );
        Ok(PhaseRequest {
            cargue_id: self.require_cargue()?.to_owned(),
            invoices: (!invoices.is_empty()).then_some(invoices),
            workers: self.workers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_and_summary_follow_the_stored_outcomes() {
        let mut report = PhaseReport::default();
        report.push("A", Outcome::done());
        report.push("B", Outcome::failed("X"));
        report.push("C", Outcome::excluded("Y"));
        report.push("D", Outcome::manual_review("Z"));
        report.skip("E", "RIPS_NO_OK");
        assert_eq!(
            report.counters(),
            Counters {
                done: 1,
                failed: 1,
                excluded: 1,
                manual_review: 1,
                skipped: 1
            }
        );
        assert!(report.has_failures());
        assert_eq!(
            report.summary(),
            "4 procesadas: 1 listas, 1 fallidas, 1 excluidas, 1 en revisión manual, 1 omitidas"
        );
    }

    #[test]
    fn common_args_build_a_normalized_request() {
        let args = CommonArgs {
            cargue: Some("C1".into()),
            invoices: vec!["fev-1".into(), "FEV1".into(), "evs2".into()],
            workers: Some(0),
        };
        let request = args.to_request().expect("request");
        assert_eq!(
            request.invoices,
            Some(vec!["FEV1".to_string(), "EVS2".to_string()])
        );
        assert_eq!(request.workers_or(10), 1);
        assert_eq!(PhaseRequest::new("C1").workers_or(10), 10);
        assert!(CommonArgs::default().to_request().is_err());
    }
}
