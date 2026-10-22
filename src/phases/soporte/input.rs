//! What the support of one invoice needs from the dataset, and the pure
//! normalization rules of the Go package `soporte` (`FromInvoice`,
//! `NormalizeSSC`, `NormalizeCostCentre`, the deposit predicates).

use std::sync::LazyLock;

use regex::Regex;

/// One dataset line of an invoice (a Medicar row), with the columns the support uses.
///
/// Phase B builds these from the dataset store; the strings are the raw
/// column values (`No. SSC`, `Pto. venta`, ...).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SoporteRow {
    /// `No. SSC` (may carry the `SSC` prefix).
    pub ssc: String,
    /// `Pto. venta` (cost centre).
    pub point_of_sale: String,
    /// `Numero autorizacion`.
    pub authorization: String,
    /// `Tipo de identificacion`.
    pub doc_type: String,
    /// `No. Documento`.
    pub doc_number: String,
    /// `Beneficiario` (patient name).
    pub beneficiary: String,
    /// `Fecha` (a timestamp; the first 10 characters are kept).
    pub date: String,
    /// `Articulo`.
    pub article: String,
    /// `Cantidad`.
    pub quantity: String,
}

/// An invoice and its dataset lines, in dataset order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SoporteInput {
    /// The normalized invoice number.
    pub invoice: String,
    pub rows: Vec<SoporteRow>,
}

/// One delivery listed on the delivery page: a line of the formula with its
/// own article, quantity, date (`yyyy-MM-dd`) and authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub article: String,
    pub quantity: String,
    pub date: String,
    pub authorization: String,
}

/// The header of the support of an invoice (Go `soporte.Input`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Header {
    pub invoice: String,
    pub ssc: String,
    /// Normalized cost centre.
    pub cost_centre: String,
    pub authorization: String,
    pub doc_type: String,
    pub doc_number: String,
    pub name: String,
    /// `yyyy-MM-dd`.
    pub date: String,
    /// Every delivery of the formula: the lines that share the first line's
    /// normalized SSC, in dataset order.
    pub items: Vec<Item>,
    /// Lines of another SSC, which are not listed in `items`.
    pub other_ssc_lines: usize,
}

impl Header {
    /// Maps an invoice the way `descargarArchivoSoporte` read the Medicar row:
    /// the header fields come from the first row; `items` lists every row that
    /// shares the first row's normalized SSC (deliberate deviation from Java,
    /// which listed the first row only), and `other_ssc_lines` counts the rest.
    pub fn from_input(input: &SoporteInput) -> Header {
        let mut header = Header {
            invoice: input.invoice.clone(),
            ..Header::default()
        };
        let Some(first) = input.rows.first() else {
            return header;
        };
        header.ssc = normalize_ssc(&first.ssc);
        header.cost_centre = normalize_cost_centre(&first.point_of_sale);
        header.authorization = first.authorization.clone();
        header.doc_type = first.doc_type.clone();
        header.doc_number = first.doc_number.clone();
        header.name = first.beneficiary.clone();
        header.date = short_date(&first.date);
        for row in &input.rows {
            if normalize_ssc(&row.ssc) != header.ssc {
                header.other_ssc_lines += 1;
                continue;
            }
            header.items.push(Item {
                article: row.article.clone(),
                quantity: row.quantity.clone(),
                date: short_date(&row.date),
                authorization: row.authorization.clone(),
            });
        }
        header
    }
}

/// The first ten characters of a timestamp (`yyyy-MM-dd`).
fn short_date(s: &str) -> String {
    s.trim().chars().take(10).collect()
}

/// Trims and removes every `SSC`/`ssc` (Go `normalizarSsc`, a single-pass replacer).
pub fn normalize_ssc(s: &str) -> String {
    let s = s.trim();
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        if rest.starts_with("SSC") || rest.starts_with("ssc") {
            rest = &rest[3..];
            continue;
        }
        let ch_len = rest.chars().next().map_or(1, char::len_utf8);
        out.push_str(&rest[..ch_len]);
        rest = &rest[ch_len..];
    }
    out.trim().to_owned()
}

static SPACES: LazyLock<Regex> = LazyLock::new(|| {
    // Go's `\s` is ASCII only.
    Regex::new(r"[\t\n\x0c\r ]+").unwrap_or_else(|_| unreachable!("constant regex"))
});

/// Go `normalizarCentroCosto`: trimmed, upper case, single spaces.
pub fn normalize_cost_centre(s: &str) -> String {
    SPACES
        .replace_all(&s.trim().to_uppercase(), " ")
        .into_owned()
}

/// Go `esDeposito`: `BODEGA`, `BLOC` or `DEPOSITO` in the cost centre.
pub fn is_deposit(ceco: &str) -> bool {
    ceco.contains("BODEGA") || ceco.contains("BLOC") || ceco.contains("DEPOSITO")
}

/// Go `esDepositoCpscanObligatorio`: the support must come from CPScan.
pub fn is_mandatory_deposit(ceco: &str) -> bool {
    ceco.contains("BODEGA")
        || (ceco.contains("DEPOSITO CARTAGENA") && ceco.contains("BLOC PORT"))
        || ceco.contains("DEPOSITO SINCELEJO")
}

/// Go `esSede002`: the cost centre is `002` or has a `002` token when split on
/// non-digits (`SEDE 002`, `CC-002-B`; not `1002`).
pub fn is_sede_002(ceco: &str) -> bool {
    let v = ceco.trim();
    v == "002" || v.split(|c: char| !c.is_ascii_digit()).any(|t| t == "002")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ssc: &str, article: &str, date: &str) -> SoporteRow {
        SoporteRow {
            ssc: ssc.into(),
            point_of_sale: "  centro   medico ".into(),
            authorization: format!("AUT-{article}"),
            doc_type: "CC".into(),
            doc_number: "123".into(),
            beneficiary: "ANA PEREZ".into(),
            date: date.into(),
            article: article.into(),
            quantity: "2".into(),
        }
    }

    #[test]
    fn ssc_loses_its_prefix_everywhere_but_keeps_other_case_mixes() {
        assert_eq!(normalize_ssc(" SSC12345 "), "12345");
        assert_eq!(normalize_ssc("ssc7"), "7");
        assert_eq!(normalize_ssc("SSCssc9"), "9");
        assert_eq!(normalize_ssc("Ssc5"), "Ssc5");
        assert_eq!(normalize_ssc("ssSSCc"), "ssc");
        assert_eq!(normalize_ssc("  "), "");
    }

    #[test]
    fn cost_centre_is_trimmed_upper_cased_and_single_spaced() {
        assert_eq!(
            normalize_cost_centre("  deposito \t cartagena\n bloc  port "),
            "DEPOSITO CARTAGENA BLOC PORT"
        );
        assert_eq!(normalize_cost_centre(""), "");
    }

    #[test]
    fn deposit_rules_are_the_hardcoded_lists() {
        assert!(is_deposit("BODEGA NORTE") && is_deposit("BLOC X") && is_deposit("DEPOSITO Y"));
        assert!(!is_deposit("CENTRO MEDICO"));
        assert!(is_mandatory_deposit("BODEGA NORTE"));
        assert!(is_mandatory_deposit("DEPOSITO CARTAGENA BLOC PORT"));
        assert!(!is_mandatory_deposit("DEPOSITO CARTAGENA"));
        assert!(!is_mandatory_deposit("BLOC PORT"));
        assert!(is_mandatory_deposit("DEPOSITO SINCELEJO"));
        assert!(is_deposit("BLOC PORT") && !is_mandatory_deposit("BLOC PORT"));
    }

    #[test]
    fn sede_002_needs_the_exact_token() {
        for yes in ["002", " 002 ", "SEDE 002", "CC-002-B", "002A"] {
            assert!(is_sede_002(yes), "{yes}");
        }
        for no in ["1002", "0020", "SEDE 2", "", "ABC"] {
            assert!(!is_sede_002(no), "{no}");
        }
    }

    #[test]
    fn header_lists_every_delivery_of_the_first_ssc_and_counts_the_others() {
        let input = SoporteInput {
            invoice: "FEV1".into(),
            rows: vec![
                row("SSC10", "A", "2026-10-01 08:00:00"),
                row("10", "B", "2026-10-02"),
                row("SSC11", "C", "2026-10-03"),
            ],
        };
        let header = Header::from_input(&input);
        assert_eq!(header.ssc, "10");
        assert_eq!(header.cost_centre, "CENTRO MEDICO");
        assert_eq!(header.date, "2026-10-01");
        assert_eq!(header.name, "ANA PEREZ");
        assert_eq!(header.items.len(), 2);
        assert_eq!(header.items[1].article, "B");
        assert_eq!(header.items[1].authorization, "AUT-B");
        assert_eq!(header.other_ssc_lines, 1);
    }

    #[test]
    fn header_of_an_invoice_without_rows_is_empty() {
        let header = Header::from_input(&SoporteInput {
            invoice: "FEV2".into(),
            rows: vec![],
        });
        assert_eq!(header.ssc, "");
        assert!(header.items.is_empty());
    }
}
