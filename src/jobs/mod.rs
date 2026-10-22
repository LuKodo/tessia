//! The in-process job runner: one job at a time, synthetic log, JSONL history.
//!
//! * [`runner`]: [`JobRunner`] (start, cancel, close) and [`JobBackend`], what
//!   each kind of job executes;
//! * [`snapshot`]: a job's live state and the polled [`JobSnapshot`];
//! * [`synthetic_log`]: the Spanish `salida`;
//! * [`history`]: the JSONL file of finished jobs.

pub mod history;
pub mod runner;
pub mod snapshot;
pub mod synthetic_log;

pub use history::HistoryEntry;
pub use runner::{
    CANCELLED_ERROR, JobBackend, JobRunner, NoJobRunning, StartError, StartOptions, Work,
};
pub use snapshot::{JobConvenio, JobSnapshot, JobStats, PhaseProgress};
