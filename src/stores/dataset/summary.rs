//! The stored summary of a prepared invoice (Go `dashboard.SummarizeLines`).
//!
//! `dataset_factura` keeps five columns derived from the lines of its invoice
//! so the dashboard can filter, sort and sum without reading `dataset_linea`:
//! `fecha_factura_txt`, `ssc`, `subplan`, `autorizacion` and `valor_convenio`.
//! [`summarize`] is the only Rust implementation of that derivation; the
//! backfill migration `2026-10-24-000200_dataset_factura_summary_backfill`
//! applies the same rules in SQL to the rows stored earlier.

use rust_decimal::Decimal;

use crate::domain::dataset::Line;
use crate::util::decimal::parse_agreement_amount;

/// Agreement totals from this value up do not fit `NUMERIC(18,2)`.
const MAX_TOTAL: i64 = 10_000_000_000_000_000;

/// The invoice-level fields derived from the lines of one prepared invoice.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InvoiceSummary {
    /// First non-blank line date, trimmed, cut to 10 characters
    /// (`"2026-09-03 11:28:31"` is `"2026-09-03"`). Not validated.
    pub date: String,
    /// First non-blank line value of each field, trimmed.
    pub ssc: String,
    pub subplan: String,
    pub authorization: String,
    /// Sum of the line totals; `None` when the invoice has no lines or any
    /// total cannot be read (one bad value blanks the whole sum).
    pub agreement_value: Option<Decimal>,
}

/// Derives the [`InvoiceSummary`] of an invoice's lines, in line order.
pub fn summarize(lines: &[Line]) -> InvoiceSummary {
    let first = |get: fn(&Line) -> &str| -> String {
        lines
            .iter()
            .map(|l| get(l).trim())
            .find(|v| !v.is_empty())
            .unwrap_or("")
            .to_owned()
    };
    InvoiceSummary {
        date: first(|l| &l.date).chars().take(10).collect(),
        ssc: first(|l| &l.ssc),
        subplan: first(|l| &l.subplan),
        authorization: first(|l| &l.authorization),
        agreement_value: agreement_value(lines),
    }
}

fn agreement_value(lines: &[Line]) -> Option<Decimal> {
    if lines.is_empty() {
        return None;
    }
    let mut total = Decimal::ZERO;
    for line in lines {
        total += parse_agreement_amount(&line.total_value).ok()?;
    }
    (total < Decimal::from(MAX_TOTAL)).then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(date: &str, ssc: &str, total: &str) -> Line {
        Line {
            date: date.into(),
            ssc: ssc.into(),
            total_value: total.into(),
            ..Line::default()
        }
    }

    #[test]
    fn takes_the_first_non_blank_value_and_cuts_the_date() {
        let mut second = line("2026-09-04", "SSC2", "9.5");
        second.subplan = " EVENTO POS ".into();
        second.authorization = "AUT".into();
        let s = summarize(&[line("  ", "", "100.50"), second]);
        assert_eq!(s.date, "2026-09-04");
        assert_eq!(s.ssc, "SSC2");
        assert_eq!(s.subplan, "EVENTO POS");
        assert_eq!(s.authorization, "AUT");
        let long = summarize(&[line("2026-09-03 11:28:31", "", "1")]);
        assert_eq!(long.date, "2026-09-03");
    }

    #[test]
    fn sums_with_the_agreement_rules() {
        let s = summarize(&[line("", "", "100.50"), line("", "", "1.234,50")]);
        assert_eq!(s.agreement_value, Some("1335.00".parse().expect("dec")));
    }

    #[test]
    fn an_unreadable_total_or_no_lines_leave_the_value_empty() {
        assert_eq!(summarize(&[]).agreement_value, None);
        let bad = summarize(&[line("", "", "10"), line("", "", "abc")]);
        assert_eq!(bad.agreement_value, None);
    }

    #[test]
    fn a_total_beyond_numeric_18_2_is_left_empty() {
        let big: Vec<Line> = (0..20_000).map(|_| line("", "", "999999999999")).collect();
        assert_eq!(summarize(&big).agreement_value, None);
    }
}
