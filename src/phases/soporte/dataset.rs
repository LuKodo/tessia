//! Adapter from the prepared convenio dataset to the phase input: the Medicar
//! columns `Go soporte.FromInvoice` reads become [`SoporteRow`]s.

use anyhow::{Context, Result, bail};

use super::input::{SoporteInput, SoporteRow};
use crate::context::AppContext;
use crate::domain::dataset::{Invoice, Line};
use crate::phases::common::PhaseRequest;

/// One dataset line as a support row (verbatim CSV text).
fn row_of(line: &Line) -> SoporteRow {
    SoporteRow {
        ssc: line.ssc.clone(),
        point_of_sale: line.point_of_sale.clone(),
        authorization: line.authorization.clone(),
        doc_type: line.doc_type.clone(),
        doc_number: line.doc_number.clone(),
        beneficiary: line.beneficiary.clone(),
        date: line.date.clone(),
        article: line.article.clone(),
        quantity: line.quantity.clone(),
    }
}

/// The input of one invoice: its lines in dataset order.
pub fn input_of(invoice: &Invoice) -> SoporteInput {
    SoporteInput {
        invoice: invoice.invoice.clone(),
        rows: invoice.lines.iter().map(row_of).collect(),
    }
}

/// The prepared invoices of the cargue's current dataset (only `req.invoices`
/// when given; the rest of the dataset is not read).
pub fn load_inputs(ctx: &AppContext, req: &PhaseRequest) -> Result<Vec<SoporteInput>> {
    let dataset = ctx
        .dataset()
        .read_current(&req.cargue_id, req.invoices.as_deref())
        .context("no se pudo leer el dataset del cargue")?;
    let Some(dataset) = dataset else {
        bail!("el cargue {} no tiene un dataset preparado", req.cargue_id);
    };
    Ok(dataset.invoices.iter().map(input_of).collect())
}
