//! Per-invoice, per-step progress of a radicación run, for the dashboard.
//!
//! The orchestrator that files a batch registers a sink for the cargue; the
//! engine looks it up when the run starts. Registering instead of passing the
//! sink through `PhaseRequest` keeps that shared request untouched.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::domain::radicacion::Step;

/// What happened to an invoice at a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepProgress {
    /// The invoice is waiting for, or going through, the step.
    Enter,
    /// The step was confirmed by Mutual Ser.
    Done,
    /// The invoice went back from the step without finishing it.
    Back,
    /// The invoice left the pipeline at the step (rejected or blocked).
    Failed,
}

impl StepProgress {
    pub fn as_str(self) -> &'static str {
        match self {
            StepProgress::Enter => "ENTER",
            StepProgress::Done => "DONE",
            StepProgress::Back => "BACK",
            StepProgress::Failed => "FAILED",
        }
    }
}

/// Told about every step transition: `(invoice, step, what happened)`. Must be
/// cheap and never panic.
pub type ProgressSink = Arc<dyn Fn(&str, Step, StepProgress) + Send + Sync>;

fn sinks() -> MutexGuard<'static, HashMap<String, ProgressSink>> {
    static SINKS: OnceLock<Mutex<HashMap<String, ProgressSink>>> = OnceLock::new();
    SINKS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Keeps a sink registered for a cargue until dropped.
pub struct Registration {
    cargue: String,
}

/// Registers `sink` for the runs of `cargue` started while the result lives.
pub fn register(cargue: &str, sink: ProgressSink) -> Registration {
    sinks().insert(cargue.to_owned(), sink);
    Registration {
        cargue: cargue.to_owned(),
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        sinks().remove(&self.cargue);
    }
}

/// The sink registered for `cargue`, if any.
pub(super) fn lookup(cargue: &str) -> Option<ProgressSink> {
    sinks().get(cargue).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sink_is_found_only_while_it_is_registered() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let registration = register(
            "progress-test-cargue",
            Arc::new(move |invoice, step, what| {
                log.lock().unwrap().push((invoice.to_owned(), step, what));
            }),
        );
        let sink = lookup("progress-test-cargue").expect("registered");
        sink("F1", Step::Cuv, StepProgress::Done);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [("F1".to_owned(), Step::Cuv, StepProgress::Done)]
        );
        assert!(lookup("another-cargue").is_none());
        drop(registration);
        assert!(lookup("progress-test-cargue").is_none());
    }
}
