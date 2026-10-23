//! Invoice number rules (port of Go `radicacion.NormalizeInvoice` and friends).

use std::sync::LazyLock;

use regex::Regex;

static INVOICE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // The pattern is a constant; failing to compile it is a programming error
    // caught by the unit tests.
    Regex::new(r"^[A-Z]{3}\d+$").unwrap_or_else(|_| unreachable!("constant regex"))
});

/// Normalized key of an invoice: trimmed, dashes removed, upper case.
///
/// This is the key of `factura_valor_xml.num_factura`,
/// `radicacion_estado.invoice` and `proceso_estado.num_factura`.
///
/// ```
/// assert_eq!(tessia::domain::normalize_invoice(" evs-12 "), "EVS12");
/// ```
pub fn normalize_invoice(raw: &str) -> String {
    raw.trim().replace('-', "").to_uppercase()
}

/// True when `s` looks like an invoice number: three letters and digits.
pub fn valid_invoice(s: &str) -> bool {
    INVOICE_RE.is_match(s)
}

/// Extracts the invoice number from a step file name such as `FEV1_XML_PDF.zip`,
/// `FEV1.json`, `FEV1.txt` or `CRC_<nit>_FEV1.zip`.
pub fn invoice_from_file_name(name: &str, nit: &str) -> Option<String> {
    let mut s = name.trim().to_uppercase();
    if !nit.is_empty() {
        s = s.replacen(&format!("CRC_{}_", nit.to_uppercase()), "", 1);
    }
    for suffix in ["_XML_PDF", ".ZIP", ".JSON", ".TXT", ".XML", ".PDF"] {
        s = s.replace(suffix, "");
    }
    valid_invoice(&s).then_some(s)
}

/// Normalizes, drops empties and removes duplicates keeping the first
/// occurrence order.
pub fn normalize_all<I, S>(invoices: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut seen = std::collections::HashSet::new();
    invoices
        .into_iter()
        .map(|i| normalize_invoice(i.as_ref()))
        .filter(|i| !i.is_empty() && seen.insert(i.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_trims_removes_dashes_and_uppercases() {
        assert_eq!(normalize_invoice("  fev-0012 "), "FEV0012");
        assert_eq!(normalize_invoice(""), "");
    }

    #[test]
    fn validity_requires_three_letters_and_digits() {
        assert!(valid_invoice("FEV123"));
        assert!(!valid_invoice("FE123"));
        assert!(!valid_invoice("FEV"));
        assert!(!valid_invoice("fev123"));
    }

    #[test]
    fn invoice_is_extracted_from_step_file_names() {
        assert_eq!(
            invoice_from_file_name("FEV1_XML_PDF.zip", "").as_deref(),
            Some("FEV1")
        );
        assert_eq!(
            invoice_from_file_name("fev1.json", "").as_deref(),
            Some("FEV1")
        );
        assert_eq!(
            invoice_from_file_name("CRC_8060_FEV1.zip", "8060").as_deref(),
            Some("FEV1")
        );
        assert_eq!(invoice_from_file_name("notes.txt", ""), None);
    }

    #[test]
    fn normalize_all_dedupes_keeping_order() {
        assert_eq!(normalize_all(["b-1", "A1", "B1", " "]), ["B1", "A1"]);
    }
}
