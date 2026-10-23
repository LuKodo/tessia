//! `ajustarPeriodoAutorizacion`: forces the start date of every
//! `AuthorizationPeriod` / `InvoicePeriod`.
//!
//! Java's pattern closes the period with a back-reference, which the `regex`
//! crate lacks, so it is matched by hand. Every lazy step can only move right
//! and the closing tag must come after all of them, so taking the first
//! occurrence of each part finds the same match a backtracking engine finds
//! (and fails where it fails).

use std::sync::LazyLock;

use regex::Regex;

use super::FevxmlError;
use super::text::{group, java_trim, re};

static PERIOD_OPEN: LazyLock<Regex> =
    LazyLock::new(|| re(r"<((?:(?-u:\w)+:)?)(AuthorizationPeriod|InvoicePeriod)(?-u:\b)[^>]*>"));
static START_OPEN: LazyLock<Regex> =
    LazyLock::new(|| re(r"<(?:(?-u:\w)+:)?StartDate(?-u:\b)[^>]*>"));
static START_CLOSE: LazyLock<Regex> = LazyLock::new(|| re(r"</(?:(?-u:\w)+:)?StartDate>"));
static END_OPEN: LazyLock<Regex> = LazyLock::new(|| re(r"<(?:(?-u:\w)+:)?EndDate(?-u:\b)[^>]*>"));
static END_CLOSE: LazyLock<Regex> = LazyLock::new(|| re(r"</(?:(?-u:\w)+:)?EndDate>"));

/// One match; fields are absolute byte offsets.
struct Period {
    end: usize,
    start_val: usize,
    start_end: usize,
}

fn find(r: &Regex, s: &str, from: usize) -> Option<(usize, usize)> {
    r.find(&s[from..])
        .map(|m| (from + m.start(), from + m.end()))
}

/// Finds `</name>` at or after `from`, ASCII case-insensitively (a Java
/// back-reference under CASE_INSENSITIVE), and returns its end.
fn close_tag(s: &str, from: usize, name: &str) -> Option<usize> {
    let needle = format!("</{name}>");
    let n = needle.as_bytes();
    let b = s.as_bytes();
    (from..=b.len().checked_sub(n.len())?)
        .find(|&i| b[i..i + n.len()].eq_ignore_ascii_case(n))
        .map(|i| i + n.len())
}

fn match_period_at(s: &str, open_end: usize, name: &str, need_end: bool) -> Option<Period> {
    let (_, so1) = find(&START_OPEN, s, open_end)?;
    let (sc0, sc1) = find(&START_CLOSE, s, so1)?;
    let mut after = sc1;
    if need_end {
        let (_, eo1) = find(&END_OPEN, s, sc1)?;
        let (_, ec1) = find(&END_CLOSE, s, eo1)?;
        after = ec1;
    }
    let end = close_tag(s, after, name)?;
    Some(Period {
        end,
        start_val: so1,
        start_end: sc0,
    })
}

/// Non-overlapping matches, scanning like `Matcher.find`.
fn periods(s: &str, need_end: bool) -> Vec<Period> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos <= s.len() {
        let Some(caps) = PERIOD_OPEN.captures(&s[pos..]) else {
            break;
        };
        let Some(whole) = caps.get(0) else { break };
        let o0 = pos + whole.start();
        let o1 = pos + whole.end();
        let name = format!("{}{}", group(&caps, 1), group(&caps, 2));
        match match_period_at(s, o1, &name, need_end) {
            Some(p) => {
                pos = p.end;
                out.push(p);
            }
            None => pos = o0 + 1,
        }
    }
    out
}

/// Forces `inicio` as start date of every period. Without periods, or with an
/// empty `inicio`, the text is unchanged. Fails with [`FevxmlError::Periodo`]
/// when a period (even one without `EndDate`) keeps another start date.
pub(crate) fn ajustar_periodo(s: &str, inicio: &str) -> Result<String, FevxmlError> {
    if inicio.is_empty() {
        return Ok(s.to_owned());
    }
    let ps = periods(s, true);
    if ps.is_empty() {
        return Ok(s.to_owned());
    }
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    let mut changed = false;
    for p in &ps {
        out.push_str(&s[last..p.start_val]);
        if java_trim(&s[p.start_val..p.start_end]) == inicio {
            out.push_str(&s[p.start_val..p.start_end]);
        } else {
            out.push_str(inicio);
            changed = true;
        }
        last = p.start_end;
    }
    out.push_str(&s[last..]);
    for p in periods(&out, false) {
        if java_trim(&out[p.start_val..p.start_end]) != inicio {
            return Err(FevxmlError::Periodo);
        }
    }
    if changed { Ok(out) } else { Ok(s.to_owned()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_start_dates_with_end_date() {
        let s = "<cac:InvoicePeriod><cbc:StartDate>2026-08-01</cbc:StartDate><cbc:EndDate>x</cbc:EndDate></cac:InvoicePeriod>";
        let out = ajustar_periodo(s, "2025-01-01").unwrap();
        assert!(out.contains(">2025-01-01</cbc:StartDate>"));
    }

    #[test]
    fn lone_period_without_end_date_is_left_alone() {
        let s = "<cac:InvoicePeriod><cbc:StartDate>2026-08-01</cbc:StartDate></cac:InvoicePeriod>";
        assert_eq!(ajustar_periodo(s, "2025-01-01").unwrap(), s);
    }

    #[test]
    fn period_without_end_date_next_to_a_full_one_fails_the_check() {
        let s = "<cac:AuthorizationPeriod><cbc:StartDate>2026-08-01</cbc:StartDate><cbc:EndDate>x</cbc:EndDate></cac:AuthorizationPeriod><cac:InvoicePeriod><cbc:StartDate>2026-08-02</cbc:StartDate></cac:InvoicePeriod>";
        assert!(matches!(
            ajustar_periodo(s, "2025-01-01"),
            Err(FevxmlError::Periodo)
        ));
    }

    #[test]
    fn closing_tag_must_match_case_insensitively() {
        assert_eq!(
            close_tag("..</CAC:invoiceperiod>", 0, "cac:InvoicePeriod"),
            Some(22)
        );
    }
}
