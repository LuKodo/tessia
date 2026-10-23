//! `XmlInvoiceLineReconciliation.invoiceLines` (CONC-05): invoice lines of a FEV
//! XML, from the document itself or from the `Invoice` embedded in a
//! `Description` CDATA (AttachedDocument).

use std::sync::LazyLock;

use regex::Regex;
use rust_decimal::Decimal;

use super::FevxmlError;
use super::ajuste::parse_decimal_literal;
use super::dom::{self, Element};
use super::text::decode;

/// One `cac:InvoiceLine` of a FEV invoice. Amounts are exact decimals (the Go
/// struct used `float64`; callers needing floats can convert).
#[derive(Debug, Clone, PartialEq)]
pub struct InvoiceLine {
    /// `StandardItemIdentification/ID`, else `SellersItemIdentification/ID`
    /// (`<PLU>-<CUM>`).
    pub identifier: String,
    /// First `Item/Description`.
    pub description: String,
    /// `InvoicedQuantity`.
    pub quantity: Decimal,
    /// `Price/PriceAmount`.
    pub unit_price: Decimal,
    /// `LineExtensionAmount`.
    pub line_total: Decimal,
}

static EMBEDDED_INVOICE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Same as Go's `(?s)<Invoice\b.*</Invoice>` (case-sensitive).
    match Regex::new(r"(?s)<Invoice(?-u:\b).*</Invoice>") {
        Ok(r) => r,
        Err(e) => panic!("fevxml: invalid built-in pattern: {e}"),
    }
});

fn decimal(s: &str) -> Option<Decimal> {
    let t = s.trim();
    if s.is_empty() {
        return None;
    }
    parse_decimal_literal(t)
}

/// Reads the invoice lines. Every line needs an identifier and a positive
/// quantity, unit price and total with `|quantity x price - total| <= 0.01`.
pub fn invoice_lines(b: &[u8]) -> Result<Vec<InvoiceLine>, FevxmlError> {
    let s = decode(b);
    let root = dom::parse_root(&s)
        .map_err(FevxmlError::XmlInvalido)?
        .ok_or_else(|| FevxmlError::XmlInvalido("sin elemento raiz".to_owned()))?;
    let mut nodes: Vec<&Element> = root.descendants("InvoiceLine");
    let inner_root;
    if nodes.is_empty() {
        for d in root.descendants("Description") {
            let Some(m) = EMBEDDED_INVOICE_RE.find(&d.text) else {
                continue;
            };
            let Ok(Some(inner)) = dom::parse_root(m.as_str()) else {
                continue;
            };
            if !inner.descendants("InvoiceLine").is_empty() {
                inner_root = inner;
                nodes = inner_root.descendants("InvoiceLine");
                break;
            }
        }
    }
    if nodes.is_empty() {
        return Err(FevxmlError::SinLineas);
    }
    let tolerance = Decimal::new(1, 2);
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        let mut id = n.first_text("StandardItemIdentification", "ID");
        if id.is_empty() {
            id = n.first_text("SellersItemIdentification", "ID");
        }
        let qty = decimal(&n.child_text("InvoicedQuantity"));
        let total = decimal(&n.child_text("LineExtensionAmount"));
        let unit = n
            .child("Price")
            .and_then(|p| decimal(&p.child_text("PriceAmount")));
        let (Some(qty), Some(total), Some(unit)) = (qty, total, unit) else {
            return Err(FevxmlError::LineaIncompleta(id));
        };
        if id.is_empty() || qty <= Decimal::ZERO || total <= Decimal::ZERO || unit <= Decimal::ZERO
        {
            return Err(FevxmlError::LineaIncompleta(id));
        }
        let consistent = qty
            .checked_mul(unit)
            .and_then(|p| p.checked_sub(total))
            .is_some_and(|d| d.abs() <= tolerance);
        if !consistent {
            return Err(FevxmlError::LineaInconsistente(id));
        }
        out.push(InvoiceLine {
            identifier: id,
            description: n.first_text("Item", "Description"),
            quantity: qty,
            unit_price: unit,
            line_total: total,
        });
    }
    Ok(out)
}
