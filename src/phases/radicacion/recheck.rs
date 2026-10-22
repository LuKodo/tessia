//! `radicacion recheck-step2`: re-verifies step 2 (TXT/CUV) of already filed
//! invoices against `filter-xml` only (Go `pipeline/recheck.go`, the legacy
//! `corregirConfirmacionesPaso2Sospechosas`).
//!
//! `statusType=3` means Mutual Ser rejected the CUV: an adjustment note is
//! recorded in `radicacion_estado`. A negative result never clears a
//! confirmed step.

use anyhow::Result;

use super::api::RadApi;
use crate::domain::normalize_invoice;
use crate::domain::radicacion::{RadState, RadStatus, Step};
use crate::stores::radicacion::RadicacionStore;
use crate::util::par;

/// What a recheck did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecheckReport {
    pub total: usize,
    pub confirmed: usize,
    pub not_confirmed: usize,
    pub rejected: usize,
    pub errors: usize,
    /// The process was asked to stop before every invoice was checked.
    pub interrupted: bool,
}

impl RecheckReport {
    /// The line the command prints.
    pub fn line(&self) -> String {
        format!(
            "total={} confirmed={} not_confirmed={} rejected={} errors={}",
            self.total, self.confirmed, self.not_confirmed, self.rejected, self.errors
        )
    }
}

#[derive(Clone, Copy)]
enum Verdict {
    Confirmed,
    NotConfirmed,
    Rejected,
    /// Rejected by Mutual Ser but the note could not be stored.
    RejectedUnsaved,
    Failed,
    Skipped,
}

/// Rechecks step 2 of `invoices`.
pub fn recheck_step2(
    api: &dyn RadApi,
    states: &RadicacionStore,
    invoices: &[String],
) -> Result<RecheckReport> {
    let invoices =
        crate::domain::invoice::normalize_all(invoices.iter().map(|i| normalize_invoice(i)));
    let stored = states.load(&invoices)?;
    let workers = invoices.len().clamp(4, 20);
    let verdicts = par::map(invoices.clone(), workers, |invoice| {
        if crate::shutdown::requested() {
            return Verdict::Skipped;
        }
        recheck_one(api, states, &stored, &invoice)
    });
    let mut report = RecheckReport {
        total: invoices.len(),
        ..RecheckReport::default()
    };
    for verdict in verdicts {
        match verdict {
            Verdict::Confirmed => report.confirmed += 1,
            Verdict::NotConfirmed => report.not_confirmed += 1,
            Verdict::Rejected => {
                report.not_confirmed += 1;
                report.rejected += 1;
            }
            Verdict::RejectedUnsaved => {
                report.not_confirmed += 1;
                report.rejected += 1;
                report.errors += 1;
            }
            Verdict::Failed => report.errors += 1,
            Verdict::Skipped => report.interrupted = true,
        }
    }
    Ok(report)
}

fn recheck_one(
    api: &dyn RadApi,
    states: &RadicacionStore,
    stored: &std::collections::HashMap<String, RadState>,
    invoice: &str,
) -> Verdict {
    let found = match api.filter_xml(invoice) {
        Ok(found) => found,
        Err(err) => {
            tracing::warn!(invoice, error = %err, "step 2 recheck failed");
            return Verdict::Failed;
        }
    };
    let mut detail = format!(
        "paso=2 filter-xml reverificacion factura={invoice} encontrado={}",
        found.is_some()
    );
    let active = found
        .as_ref()
        .is_some_and(|d| d.status.eq_ignore_ascii_case("ACTIVE"));
    if let Some(doc) = &found {
        detail.push_str(&format!(
            " billId={} status={} statusType={} id={} errores={}",
            doc.bill_id,
            doc.status,
            doc.status_type,
            doc.id,
            doc.error_codes.join(",")
        ));
    }
    detail.push_str(&format!(" confirmado={active}"));
    if active {
        return Verdict::Confirmed;
    }
    if !detail.to_uppercase().contains("STATUSTYPE=3") {
        return Verdict::NotConfirmed;
    }
    let mut state = stored
        .get(invoice)
        .cloned()
        .unwrap_or_else(|| RadState::new(invoice, Step::Cuv));
    state.step = Step::Cuv;
    state.status = RadStatus::AdjustmentNote;
    state.last_detail = detail;
    match states.save(&state) {
        Ok(()) => Verdict::Rejected,
        Err(err) => {
            tracing::error!(invoice, error = %err, "recording step 2 rejection");
            Verdict::RejectedUnsaved
        }
    }
}
