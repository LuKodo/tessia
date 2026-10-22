//! Which invoices `rips generar` looks at, read from the convenio dataset of
//! the cargue. Go source: `convenio.ReadInvoices` + the selection half of
//! `rips.MakePlan`.

use std::collections::HashSet;

use anyhow::{Result, bail};

use super::input::{RipsInput, RipsLine};
use crate::context::AppContext;
use crate::domain::dataset::{Dataset, Invoice, Line, subset_names};
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

/// Reads the selection of `req` from the dataset store: the current dataset of
/// the cargue, limited to `req.invoices` when given (Go `convenio.ReadInvoices`
/// + the selection half of `MakePlan`).
pub fn load(ctx: &AppContext, req: &PhaseRequest) -> Result<Selection> {
    let subset = req.invoices.as_deref();
    let Some(dataset) = ctx.dataset().read_current(&req.cargue_id, subset)? else {
        bail!(
            "el cargue {} no tiene dataset preparado (ejecute `facturacion convenio preparar`)",
            req.cargue_id
        );
    };
    Ok(select(dataset, subset))
}

/// Maps a (possibly subset) dataset to candidates; `requested` names that the
/// dataset does not hold at all are `unknown`.
pub fn select(dataset: Dataset, requested: Option<&[String]>) -> Selection {
    let mut candidates = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for e in dataset.excluded {
        seen.insert(e.invoice.clone());
        candidates.push(Candidate::ConvenioNotOk {
            detail: format!("convenio EXCLUDED: {}", e.reason),
            invoice: e.invoice,
        });
    }
    for m in dataset.manual_review {
        seen.insert(m.invoice.clone());
        candidates.push(Candidate::ConvenioNotOk {
            detail: format!("convenio REVISION_MANUAL: {}", m.text()),
            invoice: m.invoice,
        });
    }
    for invoice in dataset.invoices {
        seen.insert(invoice.invoice.clone());
        candidates.push(Candidate::Prepared(rips_input(invoice)));
    }
    let mut unknown: Vec<String> = requested
        .map(subset_names)
        .unwrap_or_default()
        .into_iter()
        .filter(|name| !seen.contains(name))
        .collect();
    unknown.sort();
    Selection {
        candidates,
        unknown,
    }
}

fn rips_input(invoice: Invoice) -> RipsInput {
    RipsInput {
        invoice: invoice.invoice,
        lines: invoice.lines.into_iter().map(rips_line).collect(),
    }
}

fn rips_line(l: Line) -> RipsLine {
    RipsLine {
        source_line: l.source_line,
        doc_type: l.doc_type,
        doc_number: l.doc_number,
        date: l.date,
        article: l.article,
        quantity: l.quantity,
        unit_value: l.unit_value,
        total_value: l.total_value,
        copay: l.copay,
        authorization: l.authorization,
        mipres: l.mipres,
        formula_number: l.formula_number,
        diagnosis_code: l.diagnosis_code,
        prescriber_id: l.prescriber_id,
        cum: l.cum,
        product_type: l.product_type,
        pharmaceutical_form: l.pharmaceutical_form,
        concentration: l.concentration,
        measure_unit: l.measure_unit,
    }
}
