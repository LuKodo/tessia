//! `fev verificar` (FEV-05..07): compares the XML total of every invoice with the
//! sum of its convenio lines. It writes no artifact; it records phase `fev_valor`
//! (`OK` -> `DONE`, `VALOR_DIFERENTE` -> `REVISION_MANUAL`, `XML_SIN_VALOR` ->
//! `FAILED`) and the XML totals in `factura_valor_xml`.
//!
//! The invoices come from the prepared dataset ([`load_convenio_invoices`]). The comparison is pure ([`compare`]) and works on [`ConvenioInvoice`], the
//! text of the convenio `vlr. total` of each line. [`verify`] is the flow around
//! it (claims, reading the XML from the artifacts, storing the results).

use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use rust_decimal::Decimal;

use super::ajustar::SKIP_OTHER_MACHINE;
use super::xmlsrc::XmlSources;
use crate::context::AppContext;
use crate::domain::{Outcome, Phase, PhaseState};
use crate::fevxml;
use crate::phases::common::{PhaseReport, PhaseRequest};
use crate::stores::proceso::ClaimItem;
use crate::stores::valor_xml::XmlValueUpsert;
use crate::util::par;

/// Default worker threads (Go checked the invoices one after another).
const DEFAULT_WORKERS: usize = 8;

/// Reason stored with `XML_SIN_VALOR` (a stored contract).
const REASON_SIN_VALOR: &str = "XML_SIN_VALOR (no se pudo extraer el valor total del XML)";

/// One invoice of the convenio dataset, as `fev verificar` needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvenioInvoice {
    /// Normalized invoice number (also the XML file name).
    pub invoice: String,
    /// The text of the `vlr. total` of every line.
    pub lines: Vec<String>,
}

/// Go `convenioTotal`: the sum of the lines' `vlr. total`; `None` when a value
/// cannot be read or there are no lines.
pub fn convenio_total(lines: &[String]) -> Option<Decimal> {
    let mut total = Decimal::ZERO;
    for line in lines {
        total = total.checked_add(fevxml::numero_convenio(line)?)?;
    }
    (!lines.is_empty()).then_some(total)
}

/// Result of comparing the XML with the convenio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// |XML - convenio| < 0.01.
    Ok { xml: Decimal, convenio: Decimal },
    /// The totals differ (`convenio` is `None`: `NO_DISPONIBLE`).
    Different {
        xml: Decimal,
        convenio: Option<Decimal>,
    },
    /// The XML is missing or has no readable total.
    NoXmlValue,
}

/// Go `fev verificar` rule for one invoice.
pub fn compare(xml: Option<Decimal>, lines: &[String]) -> Verdict {
    let Some(xml) = xml else {
        return Verdict::NoXmlValue;
    };
    let convenio = convenio_total(lines);
    let tolerance = Decimal::new(1, 2);
    match convenio {
        Some(c) if xml.checked_sub(c).is_some_and(|d| d.abs() < tolerance) => {
            Verdict::Ok { xml, convenio: c }
        }
        _ => Verdict::Different { xml, convenio },
    }
}

impl Verdict {
    /// The `fev_valor` outcome.
    pub fn outcome(&self) -> Outcome {
        match self {
            Self::Ok { .. } => Outcome::done(),
            Self::Different { .. } => Outcome::manual_review(self.different_reason()),
            Self::NoXmlValue => Outcome::failed(REASON_SIN_VALOR),
        }
    }

    fn different_reason(&self) -> String {
        match self {
            Self::Different { xml, convenio } => format!(
                "VALOR_DIFERENTE XML={xml} convenio={}",
                convenio.map_or_else(|| "NO_DISPONIBLE".to_owned(), |c| c.to_string())
            ),
            _ => String::new(),
        }
    }

    /// The console line of the invoice.
    pub fn line(&self, invoice: &str) -> String {
        match self {
            Self::Ok { xml, convenio } => format!("  {invoice} OK XML={xml} convenio={convenio}"),
            Self::Different { .. } => format!("  {invoice} {}", self.different_reason()),
            Self::NoXmlValue => {
                format!("  {invoice} XML_SIN_VALOR (no se pudo extraer el valor total del XML)")
            }
        }
    }
}

/// What a run of `fev verificar` did.
#[derive(Debug, Default)]
pub struct VerificarRun {
    /// Console lines in Go's order (claim warnings, one per invoice, summary).
    pub lines: Vec<String>,
    pub report: PhaseReport,
    pub ok: usize,
    pub different: usize,
    pub no_xml: usize,
}

impl VerificarRun {
    /// Invoices that are not `OK`: the command exits non-zero when this is > 0.
    pub fn mismatches(&self) -> usize {
        self.different + self.no_xml
    }
}

struct Checked {
    invoice: String,
    version: i64,
    verdict: Verdict,
    stored: Option<XmlValueUpsert>,
}

fn check(sources: &XmlSources, invoice: &ConvenioInvoice, version: i64) -> Checked {
    let read = sources.read(&invoice.invoice);
    let xml_total = read.verify_xml.as_deref().and_then(fevxml::read_total);
    Checked {
        invoice: invoice.invoice.clone(),
        version,
        verdict: compare(xml_total, &invoice.lines),
        stored: read.upsert(&invoice.invoice),
    }
}

/// The prepared invoices of the cargue as `fev verificar` needs them (Go
/// `convenio.ReadInvoices`): only `req.invoices` when given, with the verbatim
/// `vlr. total` text of every line. Fails when the cargue has no prepared dataset
/// or none of the requested invoices is in it.
pub fn load_convenio_invoices(
    ctx: &AppContext,
    req: &PhaseRequest,
) -> Result<Vec<ConvenioInvoice>> {
    let Some(dataset) = ctx
        .dataset()
        .read_current(&req.cargue_id, req.invoices.as_deref())?
    else {
        bail!(
            "convenio: no hay dataset preparado para el cargue {:?} en la base de datos (ejecute `facturacion convenio preparar` primero)",
            req.cargue_id
        );
    };
    let invoices: Vec<ConvenioInvoice> = dataset
        .invoices
        .into_iter()
        .map(|inv| ConvenioInvoice {
            invoice: inv.invoice,
            lines: inv.lines.into_iter().map(|l| l.total_value).collect(),
        })
        .collect();
    if invoices.is_empty() && req.invoices.is_some() {
        bail!(
            "ninguna de las facturas indicadas está preparada en el cargue {:?}",
            req.cargue_id
        );
    }
    Ok(invoices)
}

/// `fev verificar` for `invoices` (the dataset invoices of the cargue). Every run
/// re-checks every invoice; `req.invoices` keeps only those.
pub fn verify(
    ctx: &AppContext,
    req: &PhaseRequest,
    invoices: &[ConvenioInvoice],
) -> Result<VerificarRun> {
    let wanted: Option<HashSet<&str>> = req
        .invoices
        .as_ref()
        .map(|list| list.iter().map(String::as_str).collect());
    let candidates: Vec<&ConvenioInvoice> = invoices
        .iter()
        .filter(|ci| {
            wanted
                .as_ref()
                .is_none_or(|w| w.contains(ci.invoice.as_str()))
        })
        .collect();
    let mut run = VerificarRun::default();
    let claimed = claim(ctx, req, &candidates, &mut run)?;
    let sources = XmlSources::load(&ctx.artifacts(), &req.cargue_id)?;
    let work: Vec<(&ConvenioInvoice, i64)> = candidates
        .iter()
        .filter_map(|ci| claimed.get(&ci.invoice).map(|v| (*ci, *v)))
        .collect();
    let checked = par::map(work, req.workers_or(DEFAULT_WORKERS), |(ci, version)| {
        check(&sources, ci, version)
    });
    for item in &checked {
        match item.verdict {
            Verdict::Ok { .. } => run.ok += 1,
            Verdict::Different { .. } => run.different += 1,
            Verdict::NoXmlValue => run.no_xml += 1,
        }
        run.lines.push(item.verdict.line(&item.invoice));
    }
    run.lines.push(format!(
        "fev verificar: ok={} diferentes={} sin_valor_xml={}",
        run.ok, run.different, run.no_xml
    ));
    persist_values(ctx, &checked);
    store_outcomes(ctx, &checked, &mut run)?;
    Ok(run)
}

fn claim(
    ctx: &AppContext,
    req: &PhaseRequest,
    candidates: &[&ConvenioInvoice],
    run: &mut VerificarRun,
) -> Result<HashMap<String, i64>> {
    let names: Vec<String> = candidates.iter().map(|ci| ci.invoice.clone()).collect();
    if names.is_empty() {
        return Ok(HashMap::new());
    }
    let store = ctx.proceso();
    let loaded = store.load(Phase::FevValor, Some(&names))?;
    let items: Vec<ClaimItem> = names
        .iter()
        .map(|inv| ClaimItem::from_loaded(inv, loaded.get(inv)))
        .collect();
    let claimed =
        store.claim_available(Phase::FevValor, &items, &ctx.claim_options(&req.cargue_id))?;
    for invoice in names.iter().filter(|inv| !claimed.contains_key(*inv)) {
        let running = loaded
            .get(invoice)
            .is_some_and(|s| s.state == PhaseState::EnProgreso);
        run.lines.push(if running {
            format!("warning: {invoice} is already in execution for phase fev_valor; skipping it")
        } else {
            format!("warning: claim rejected for {invoice} in phase fev_valor; skipping it")
        });
        run.report.skip(invoice, SKIP_OTHER_MACHINE);
    }
    Ok(claimed)
}

/// Go `persistXMLValues`: best effort, never part of the outcome.
fn persist_values(ctx: &AppContext, checked: &[Checked]) {
    let values: Vec<XmlValueUpsert> = checked.iter().filter_map(|c| c.stored.clone()).collect();
    if values.is_empty() {
        return;
    }
    if let Err(err) = ctx.valor_xml().upsert_xml_values(&values) {
        tracing::warn!(error = %err, "xml total not stored");
    }
}

fn store_outcomes(ctx: &AppContext, checked: &[Checked], run: &mut VerificarRun) -> Result<()> {
    let store = ctx.proceso();
    for item in checked {
        let outcome = item.verdict.outcome();
        if store.save_outcome(Phase::FevValor, &item.invoice, &outcome, item.version)? {
            run.report.push(&item.invoice, outcome);
        } else {
            run.report.skip(&item.invoice, SKIP_OTHER_MACHINE);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    fn dec(s: &str) -> Decimal {
        s.parse().expect("decimal")
    }

    #[test]
    fn convenio_total_sums_money_texts() {
        assert_eq!(
            convenio_total(&lines(&["$ 1.234,56", "100", "0,44"])),
            Some(dec("1335.00"))
        );
        assert_eq!(convenio_total(&lines(&["$1,5"])), Some(dec("1.5")));
    }

    #[test]
    fn one_unreadable_value_or_no_lines_is_not_available() {
        assert_eq!(convenio_total(&lines(&["10", "abc"])), None);
        assert_eq!(convenio_total(&lines(&["10", ""])), None);
        assert_eq!(convenio_total(&[]), None);
    }

    #[test]
    fn compare_is_ok_below_one_cent() {
        let v = compare(Some(dec("100.00")), &lines(&["60", "40.009"]));
        assert_eq!(
            v,
            Verdict::Ok {
                xml: dec("100.00"),
                convenio: dec("100.009")
            }
        );
        assert_eq!(v.line("FEV1"), "  FEV1 OK XML=100.00 convenio=100.009");
        assert_eq!(v.outcome(), Outcome::done());
    }

    #[test]
    fn one_cent_apart_is_different() {
        let v = compare(Some(dec("100.00")), &lines(&["100.01"]));
        assert_eq!(
            v.outcome(),
            Outcome::manual_review("VALOR_DIFERENTE XML=100.00 convenio=100.01")
        );
        assert_eq!(
            v.line("FEV1"),
            "  FEV1 VALOR_DIFERENTE XML=100.00 convenio=100.01"
        );
    }

    #[test]
    fn unreadable_convenio_is_different_with_no_disponible() {
        let v = compare(Some(dec("50")), &lines(&["x"]));
        assert_eq!(
            v.outcome(),
            Outcome::manual_review("VALOR_DIFERENTE XML=50 convenio=NO_DISPONIBLE")
        );
    }

    #[test]
    fn missing_xml_value_fails() {
        let v = compare(None, &lines(&["10"]));
        assert_eq!(
            v.outcome(),
            Outcome::failed("XML_SIN_VALOR (no se pudo extraer el valor total del XML)")
        );
        assert_eq!(
            v.line("FEV1"),
            "  FEV1 XML_SIN_VALOR (no se pudo extraer el valor total del XML)"
        );
    }
}
