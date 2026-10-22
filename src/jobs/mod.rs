//! The in-process job runner: one job at a time, synthetic log, JSONL history.

use crate::context::AppContext;

/// Placeholder until the job runner task lands.
pub struct JobRunner {
    pub ctx: AppContext,
}

impl JobRunner {
    pub fn new(ctx: AppContext) -> Self {
        Self { ctx }
    }
}
