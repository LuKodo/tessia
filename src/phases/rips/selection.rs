//! Which invoices `rips generar` looks at, read from the convenio dataset of
//! the cargue. Go source: `convenio.ReadInvoices` + the selection half of
//! `rips.MakePlan`.

use anyhow::{Result, bail};

use super::input::RipsInput;
use crate::context::AppContext;
use crate::phases::common::PhaseRequest;

/// An invoice of the dataset and what the convenio phase decided about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Candidate {
    /// The convenio phase excluded the invoice or sent it to manual review
    /// (Go `ds.Excluded` / `ds.ManualReview`). `detail` is
    /// `convenio EXCLUDED: <reason>` or `convenio REVISION_MANUAL: <code>: <reason>`.
    ConvenioNotOk { invoice: String, detail: String },
    /// A prepared invoice with its lines.
    Prepared(RipsInput),
}

/// The invoices to consider for a request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// Dataset order: excluded, manual review, then prepared invoices (only the
    /// requested ones when `req.invoices` is set).
    pub candidates: Vec<Candidate>,
    /// Requested invoices that are not in the dataset at all (normalized,
    /// sorted).
    pub unknown: Vec<String>,
}

/// Reads the selection of `req` from the dataset store.
pub fn load(_ctx: &AppContext, _req: &PhaseRequest) -> Result<Selection> {
    bail!("rips: la lectura del dataset del cargue aún no está disponible")
}
