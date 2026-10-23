//! Locating the invoice XML of a workdir and reading its total (dashboard,
//! `seguimiento`, `fev verificar`). The Go package read from the invoice folder;
//! the Rust workdir recreates the same layout (`XML/<F>_XML_PDF.zip`,
//! `ZIP/<F>_XML_PDF.zip`, `<F>.xml`).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use super::ajuste::read_total;
use super::zipio::first_xml_entry;

/// What [`invoice_total`] found for an invoice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmlState {
    /// No non-empty `<F>_XML_PDF.zip` and no non-empty `<F>.xml`.
    Missing,
    /// The file exists but yields no readable total.
    Unparsable,
    /// The file yields a total.
    Found,
}

/// The file an invoice XML total was (or would be) read from.
#[derive(Debug, Clone)]
pub struct InvoiceXmlFile {
    pub path: PathBuf,
    /// `<F>_XML_PDF.zip` (first `.xml` entry) rather than a loose `<F>.xml`.
    pub is_zip: bool,
    pub modified: Option<SystemTime>,
    pub size: u64,
}

/// Places `<F>_XML_PDF.zip` is kept: `<dir>/XML` first, then the older
/// `<dir>/ZIP`. New files go to the first one.
pub fn xml_pdf_zip_paths(dir: &Path, invoice: &str) -> [PathBuf; 2] {
    let name = format!("{invoice}_XML_PDF.zip");
    [dir.join("XML").join(&name), dir.join("ZIP").join(name)]
}

fn non_empty_file(p: &Path) -> Option<fs::Metadata> {
    fs::metadata(p).ok().filter(|m| m.is_file() && m.len() > 0)
}

/// First non-empty `<F>_XML_PDF.zip`, if any.
pub fn existing_xml_pdf_zip(dir: &Path, invoice: &str) -> Option<PathBuf> {
    xml_pdf_zip_paths(dir, invoice)
        .into_iter()
        .find(|p| non_empty_file(p).is_some())
}

/// Finds the file the invoice XML total comes from: the first non-empty
/// `<F>_XML_PDF.zip`, else a non-empty `<dir>/<F>.xml`. It only stats.
pub fn locate_invoice_xml(dir: &Path, invoice: &str) -> Option<InvoiceXmlFile> {
    let (path, is_zip) = match existing_xml_pdf_zip(dir, invoice) {
        Some(p) => (p, true),
        None => (dir.join(format!("{invoice}.xml")), false),
    };
    let meta = non_empty_file(&path)?;
    Some(InvoiceXmlFile {
        path,
        is_zip,
        modified: meta.modified().ok(),
        size: meta.len(),
    })
}

impl InvoiceXmlFile {
    /// Reads the file and extracts the invoice total ([`read_total`]). A ZIP is
    /// read through its first `.xml` entry only: a ZIP without one is not
    /// completed with a loose `<F>.xml`.
    pub fn total(&self) -> Option<Decimal> {
        let content = if self.is_zip {
            let bytes = fs::read(&self.path).ok()?;
            first_xml_entry(&bytes).ok().flatten()?
        } else {
            fs::read(&self.path).ok()?
        };
        read_total(&content)
    }
}

/// Locates the invoice XML and extracts its total. The file is returned for
/// [`XmlState::Found`] and [`XmlState::Unparsable`].
pub fn invoice_total(
    dir: &Path,
    invoice: &str,
) -> (Option<InvoiceXmlFile>, Option<Decimal>, XmlState) {
    let Some(f) = locate_invoice_xml(dir, invoice) else {
        return (None, None, XmlState::Missing);
    };
    match f.total() {
        Some(t) => (Some(f), Some(t), XmlState::Found),
        None => (Some(f), None, XmlState::Unparsable),
    }
}

/// How the dashboard shows the XML total (`valor_xml`): plain notation without
/// trailing zeros (`"1234.5"`, `"700"`).
pub fn format_total(d: Decimal) -> String {
    d.normalize().to_string()
}

/// The total as `factura_valor_xml` keeps it, DECIMAL(18,2): two decimals,
/// half up (`"1234.50"`, `"12.35"`).
pub fn storage_amount(d: Decimal) -> String {
    let mut r = d.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    r.rescale(2);
    r.to_string()
}
