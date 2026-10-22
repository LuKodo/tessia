//! What each invoice already has in the artifact index (TF-07, TF-14).
//!
//! The Go command looked at files in the invoice folder; here the "folder" is
//! the cargue's artifact index: `<F>.xml` non-empty, `<F>.pdf` larger than
//! 1024 bytes, `XML/<F>_XML_PDF.zip` (or the older `ZIP/...`) non-empty, and,
//! when the invoice has an SSC, a usable CPScan sidecar (see
//! [`CpscanSidecar`]) with its payload.

use std::collections::HashMap;

use anyhow::Result;

use super::cpscan::CpscanSidecar;
use crate::storage::Artifacts;
use crate::util::par;

/// An invoice to download and the SSC of its CPScan lookup (the `No. SSC` of the
/// first dataset line); no SSC = no CPScan part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TfInput {
    /// Normalized invoice number.
    pub invoice: String,
    pub ssc: Option<String>,
}

impl TfInput {
    pub fn new(invoice: &str, ssc: Option<&str>) -> Self {
        Self {
            invoice: invoice.to_owned(),
            ssc: ssc
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        }
    }
}

/// The parts an invoice already has.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    pub invoice: String,
    /// The CPScan lookup key (`None`: no CPScan part).
    pub ssc: Option<String>,
    pub has_xml: bool,
    pub has_pdf: bool,
    pub has_zip: bool,
    /// The lookup is in the index (a sidecar, and its payload when it names one).
    pub has_cpscan: bool,
    /// `CPSCAN sin soporte (...)` of a settled "CPScan has nothing" lookup,
    /// `""` otherwise: the reason a re-run records instead of losing it.
    pub stored_warning: String,
}

impl Plan {
    /// The invoice still needs its CPScan lookup.
    pub fn wants_cpscan(&self) -> bool {
        self.ssc.is_some() && !self.has_cpscan
    }

    /// Every part the plan covers is there.
    pub fn complete(&self) -> bool {
        self.has_xml && self.has_pdf && self.has_zip && !self.wants_cpscan()
    }
}

/// Plans every input, reading the sidecars of the invoices that have one.
pub(crate) fn build(
    artifacts: &Artifacts,
    cargue_id: &str,
    inputs: Vec<TfInput>,
    workers: usize,
) -> Result<Vec<Plan>> {
    let sizes: HashMap<String, i64> = artifacts
        .list(cargue_id)?
        .into_iter()
        .map(|a| (a.rel_path, a.size))
        .collect();
    Ok(par::map(inputs, workers, |input| {
        plan_one(artifacts, cargue_id, &sizes, input)
    }))
}

fn plan_one(
    artifacts: &Artifacts,
    cargue_id: &str,
    sizes: &HashMap<String, i64>,
    input: TfInput,
) -> Plan {
    let inv = &input.invoice;
    let size = |rel: &str| sizes.get(rel).copied().unwrap_or(0);
    let zip = format!("{inv}_XML_PDF.zip");
    let mut plan = Plan {
        has_xml: size(&format!("{inv}.xml")) > 0,
        has_pdf: size(&format!("{inv}.pdf")) > 1024,
        has_zip: size(&format!("XML/{zip}")) > 0 || size(&format!("ZIP/{zip}")) > 0,
        has_cpscan: false,
        stored_warning: String::new(),
        ssc: input.ssc.clone(),
        invoice: input.invoice.clone(),
    };
    if plan.ssc.is_some()
        && let Some(sidecar) = read_sidecar(artifacts, cargue_id, sizes, inv)
    {
        plan.has_cpscan = true;
        plan.stored_warning = sidecar.no_support_warning().unwrap_or_default();
    }
    plan
}

/// The sidecar when the lookup is on record and usable: it is indexed, parses,
/// and its payload (when it names one) is indexed and not empty. Anything else
/// (missing, corrupt, other version, payload gone) means "not done".
fn read_sidecar(
    artifacts: &Artifacts,
    cargue_id: &str,
    sizes: &HashMap<String, i64>,
    invoice: &str,
) -> Option<CpscanSidecar> {
    let rel = CpscanSidecar::sidecar_path(invoice);
    sizes.get(&rel)?;
    let bytes = artifacts.fetch_bytes(cargue_id, &rel).ok()?;
    let sidecar = CpscanSidecar::parse(&bytes).ok()?;
    if sidecar.has_file() {
        let payload = CpscanSidecar::payload_path(invoice, &sidecar.ext);
        if sizes.get(&payload).copied().unwrap_or(0) <= 0 {
            return None;
        }
    }
    Some(sidecar)
}
