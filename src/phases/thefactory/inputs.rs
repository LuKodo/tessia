//! Where the invoices and their SSC come from.
//!
//! PHASE A STUB: the dataset store is not merged yet. With explicit invoices
//! the SSC is unknown (the CPScan part is skipped, as Go does when the dataset
//! cannot be read); without them the command cannot know the invoices.

use anyhow::{Result, bail};

use super::plan::TfInput;
use crate::context::AppContext;
use crate::phases::common::PhaseRequest;

/// The inputs of a request and, when the SSC could not be resolved, the note to
/// print (`thefactory: dataset not readable, the CPScan part is skipped (...)`).
pub(crate) fn resolve(
    _ctx: &AppContext,
    req: &PhaseRequest,
) -> Result<(Vec<TfInput>, Option<String>)> {
    match &req.invoices {
        Some(invoices) => Ok((
            invoices.iter().map(|i| TfInput::new(i, None)).collect(),
            Some("el dataset del cargue no está disponible".to_owned()),
        )),
        None => bail!("el dataset del cargue no está disponible (o pase --invoices)"),
    }
}
