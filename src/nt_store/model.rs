use std::path::PathBuf;

use crate::services::nt_service::NtImportSummary;
use crate::timing::StepTimings;

/// Result of a completed `nt import`.
#[derive(Debug)]
pub struct NtImportReport {
    pub version: String,
    /// The current Nota Técnica database.
    pub database_path: PathBuf,
    /// Where the previous version's database was archived, if there was one.
    pub archived_database: Option<PathBuf>,
    pub summary: NtImportSummary,
    pub timings: StepTimings,
}
