//! Where the invoices and their SSC come from: the prepared dataset.
//!
//! Go read the dataset of the load folder also with `--invoices` (the SSC of an
//! invoice is the `No. SSC` of its first line, `soporte.FromInvoice`). An
//! invoice that is not in the dataset, has no SSC, or a dataset that cannot be
//! read only loses its CPScan part; without `--invoices` an unreadable dataset
//! is an error.

use std::collections::HashMap;

use anyhow::{Result, bail};

use super::plan::TfInput;
use crate::context::AppContext;
use crate::domain::dataset::Invoice;
use crate::domain::normalize_invoice;
use crate::phases::common::PhaseRequest;

/// `NormalizeSSC`: the text without any `SSC`/`ssc` marker, trimmed.
pub(crate) fn normalize_ssc(raw: &str) -> String {
    raw.trim()
        .replace("SSC", "")
        .replace("ssc", "")
        .trim()
        .to_owned()
}

/// The SSC of an invoice: the first line's, `None` when empty.
pub(crate) fn invoice_ssc(invoice: &Invoice) -> Option<String> {
    let ssc = normalize_ssc(&invoice.lines.first()?.ssc);
    (!ssc.is_empty()).then_some(ssc)
}

/// The inputs of a request and, when the SSC could not be resolved, the note to
/// print (`thefactory: dataset not readable, the CPScan part is skipped (..)`).
pub(crate) fn resolve(
    ctx: &AppContext,
    req: &PhaseRequest,
) -> Result<(Vec<TfInput>, Option<String>)> {
    let read = ctx
        .dataset()
        .read_current(&req.cargue_id, req.invoices.as_deref());
    let explicit = match &req.invoices {
        Some(list) => list,
        None => {
            return match read {
                Ok(Some(ds)) => Ok((ds.invoices.iter().map(input_of).collect(), None)),
                Ok(None) => bail!(
                    "el cargue {} no tiene un dataset preparado (o pase --invoices)",
                    req.cargue_id
                ),
                Err(err) => bail!("{err} (o pase --invoices)"),
            };
        }
    };
    match read {
        Ok(Some(ds)) => {
            let sscs: HashMap<&str, Option<String>> = ds
                .invoices
                .iter()
                .map(|i| (i.invoice.as_str(), invoice_ssc(i)))
                .collect();
            let inputs = explicit
                .iter()
                .map(|name| {
                    let name = normalize_invoice(name);
                    let ssc = sscs.get(name.as_str()).cloned().flatten();
                    TfInput::new(&name, ssc.as_deref())
                })
                .collect();
            Ok((inputs, None))
        }
        Ok(None) => Ok((
            without_ssc(explicit),
            Some(format!(
                "el cargue {} no tiene un dataset preparado",
                req.cargue_id
            )),
        )),
        Err(err) => Ok((without_ssc(explicit), Some(err.to_string()))),
    }
}

fn input_of(invoice: &Invoice) -> TfInput {
    TfInput::new(&invoice.invoice, invoice_ssc(invoice).as_deref())
}

fn without_ssc(names: &[String]) -> Vec<TfInput> {
    names
        .iter()
        .map(|n| TfInput::new(&normalize_invoice(n), None))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssc_markers_are_stripped() {
        assert_eq!(normalize_ssc(" SSC 123 "), "123");
        assert_eq!(normalize_ssc("ssc-9"), "-9");
        assert_eq!(normalize_ssc("  "), "");
    }
}
