//! Radicación (Mutual Ser filing) state machine types, ported from Go
//! `internal/radicacion`. The store is `stores::radicacion`.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};

/// One of the four filing steps, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Step {
    /// ZIP with the electronic invoice XML and PDF.
    InvoiceZip = 1,
    /// TXT with the ministry validation code (CUV).
    Cuv = 2,
    /// RIPS JSON.
    Rips = 3,
    /// ZIP CRC (delivery support).
    Crc = 4,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::InvoiceZip, Step::Cuv, Step::Rips, Step::Crc];

    pub fn as_i16(self) -> i16 {
        self as i16
    }

    pub fn from_i16(n: i16) -> Option<Step> {
        Step::ALL.into_iter().find(|s| s.as_i16() == n)
    }

    /// The following step, `None` after the last.
    pub fn next(self) -> Option<Step> {
        Step::from_i16(self.as_i16() + 1)
    }

    /// Human name used in logs and observations.
    pub fn label(self) -> &'static str {
        match self {
            Step::InvoiceZip => "ZIP XML/PDF",
            Step::Cuv => "TXT CUV",
            Step::Rips => "JSON",
            Step::Crc => "ZIP CRC",
        }
    }
}

/// State of an invoice at its current step (`radicacion_estado.status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RadStatus {
    /// The step is ready to be sent (possibly after `next_attempt_at`).
    Pending,
    /// Uploaded, awaiting confirmation.
    Sent,
    /// At step 4 the invoice is fully filed.
    Confirmed,
    /// The retry budget for the step was exhausted.
    Rejected,
    /// Mutual Ser requires an adjustment note.
    AdjustmentNote,
    /// Rejected/expired: a credit note is required.
    CreditNote,
    /// A local pre-validation failed (value mismatch, missing file).
    Blocked,
}

impl RadStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RadStatus::Pending => "PENDING",
            RadStatus::Sent => "SENT",
            RadStatus::Confirmed => "CONFIRMED",
            RadStatus::Rejected => "REJECTED",
            RadStatus::AdjustmentNote => "ADJUSTMENT_NOTE",
            RadStatus::CreditNote => "CREDIT_NOTE",
            RadStatus::Blocked => "BLOCKED",
        }
    }
}

impl fmt::Display for RadStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RadStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        [
            RadStatus::Pending,
            RadStatus::Sent,
            RadStatus::Confirmed,
            RadStatus::Rejected,
            RadStatus::AdjustmentNote,
            RadStatus::CreditNote,
            RadStatus::Blocked,
        ]
        .into_iter()
        .find(|st| st.as_str() == s)
        .ok_or_else(|| format!("estado de radicación desconocido: {s}"))
    }
}

/// Persisted pipeline state of one invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadState {
    /// Normalized invoice (see `domain::normalize_invoice`).
    pub invoice: String,
    pub folder: String,
    pub step: Step,
    pub status: RadStatus,
    /// Mutual Ser upload id of the current step (when SENT).
    pub upload_id: String,
    pub sent_at: Option<DateTime<Utc>>,
    /// Uploads of the current step.
    pub sends: i32,
    /// Confirmation queries.
    pub polls: i32,
    /// Step 1 was re-sent to recover a step 2 failure.
    pub step1_reloaded: bool,
    /// A step 1 re-send done only to recover step 2.
    pub reload_for_step2: bool,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub last_detail: String,
    /// Set by the store when reading.
    pub updated_at: Option<DateTime<Utc>>,
}

impl RadState {
    /// A fresh PENDING state at `step`.
    pub fn new(invoice: &str, step: Step) -> Self {
        Self {
            invoice: invoice.to_owned(),
            folder: String::new(),
            step,
            status: RadStatus::Pending,
            upload_id: String::new(),
            sent_at: None,
            sends: 0,
            polls: 0,
            step1_reloaded: false,
            reload_for_step2: false,
            next_attempt_at: None,
            last_detail: String::new(),
            updated_at: None,
        }
    }

    /// Fully filed: step 4 confirmed.
    pub fn is_done(&self) -> bool {
        self.step == Step::Crc && self.status == RadStatus::Confirmed
    }

    /// The pipeline must not touch the invoice again.
    pub fn is_terminal(&self) -> bool {
        match self.status {
            RadStatus::AdjustmentNote
            | RadStatus::CreditNote
            | RadStatus::Blocked
            | RadStatus::Rejected => true,
            RadStatus::Confirmed => self.step == Step::Crc,
            RadStatus::Pending | RadStatus::Sent => false,
        }
    }

    /// Done, or an adjustment/credit note: must not be reprocessed (Go
    /// `radicacion.Finished`). BLOCKED and REJECTED are retried, so they are
    /// not finished.
    pub fn is_finished(&self) -> bool {
        self.is_done()
            || matches!(
                self.status,
                RadStatus::AdjustmentNote | RadStatus::CreditNote
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_ordered_and_chained() {
        assert_eq!(Step::InvoiceZip.next(), Some(Step::Cuv));
        assert_eq!(Step::Crc.next(), None);
        assert_eq!(Step::from_i16(3), Some(Step::Rips));
        assert_eq!(Step::from_i16(0), None);
    }

    #[test]
    fn finished_matches_go() {
        let mut s = RadState::new("FEV1", Step::Crc);
        s.status = RadStatus::Confirmed;
        assert!(s.is_done() && s.is_finished() && s.is_terminal());
        s.step = Step::Rips;
        assert!(!s.is_finished() && !s.is_terminal());
        s.status = RadStatus::CreditNote;
        assert!(s.is_finished());
        s.status = RadStatus::Blocked;
        assert!(!s.is_finished() && s.is_terminal());
        s.status = RadStatus::Rejected;
        assert!(!s.is_finished() && s.is_terminal());
    }

    #[test]
    fn status_names_round_trip() {
        for st in [
            "PENDING",
            "SENT",
            "CONFIRMED",
            "REJECTED",
            "ADJUSTMENT_NOTE",
            "CREDIT_NOTE",
            "BLOCKED",
        ] {
            assert_eq!(st.parse::<RadStatus>().map(RadStatus::as_str), Ok(st));
        }
    }
}
