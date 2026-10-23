//! Console summary of a preparation (port of Go `printPreparation`). The
//! lines are Go's, unchanged except the nota técnica line (the NT is a Redis
//! version now).

use std::collections::HashSet;

use super::prepare::PrepareResult;
use crate::domain::cargue::LoadInfo;

/// What the summary needs.
pub struct SummaryInput<'a> {
    pub csv_name: &'a str,
    pub load: &'a LoadInfo,
    pub nt_version: &'a str,
    pub nt_entries: usize,
    pub lines_read: usize,
    pub lines_skipped: usize,
    pub result: &'a PrepareResult,
}

/// The summary, one entry per console line.
pub fn summary_lines(input: &SummaryInput<'_>) -> Vec<String> {
    let result = input.result;
    let stats = &result.stats;
    let mut out = Vec::new();
    let mut head = format!("convenio: {} (load {}", input.csv_name, input.load.id);
    if let (Some(from), Some(to)) = (input.load.from, input.load.to) {
        head.push_str(&format!(
            ", {}..{}",
            from.format("%Y-%m-%d"),
            to.format("%Y-%m-%d")
        ));
    }
    head.push(')');
    out.push(head);
    out.push(format!(
        "nota técnica: versión {} (Redis, {} entradas cruzadas con el CSV)",
        input.nt_version, input.nt_entries
    ));
    out.push(format!(
        "lines read={} skipped={} | invoices prepared={} excluded={} manual_review={} | medicines={} other services={} copay lines={}",
        input.lines_read,
        input.lines_skipped,
        result.invoices.len(),
        result.excluded.len(),
        result.manual_review.len(),
        stats.medicines,
        stats.other_services,
        stats.copay_lines
    ));
    if stats.mipres_unavailable > 0 {
        out.push(format!(
            "{} invoices held because MIPRES was not available; the next run re-evaluates them (run convenio preparar again once MIPRES answers)",
            stats.mipres_unavailable
        ));
    }
    if result.prepared_format {
        out.push("prepared-format load detected (CLASIFICACION 714/799): drug fields come from the file; derechos validar will take the affiliate rights from the file, without querying the validator".to_owned());
    }
    if stats.finished > 0 {
        out.push(format!(
            "already finished (not counted in --limit): {}",
            stats.finished
        ));
    }
    if stats.limit > 0 {
        out.push(format!(
            "limit: {}/{} prepared (evaluated {}, excluded {}, manual_review {})",
            stats.prepared_pending,
            stats.limit,
            stats.evaluated,
            result.excluded.len(),
            result.manual_review.len()
        ));
    }
    // One line per invoice, even if a caller passes duplicated outcomes.
    let mut printed = HashSet::new();
    for review in &result.manual_review {
        if printed.insert(review.invoice.as_str()) {
            out.push(format!(
                "  manual review {}: {}",
                review.invoice,
                review.text()
            ));
        }
    }
    for excluded in &result.excluded {
        if printed.insert(excluded.invoice.as_str()) {
            out.push(format!(
                "  excluded {}: {}",
                excluded.invoice, excluded.reason
            ));
        }
    }
    out
}

/// The final line of a stored preparation.
pub fn saved_line(prepared: usize, load_id: &str) -> String {
    format!("saved {prepared} prepared invoice(s) of load {load_id:?} in the cargue tables")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::cargue::load_context;
    use crate::domain::dataset::{Excluded, ManualReview};

    #[test]
    fn the_summary_follows_the_go_lines() {
        let load = load_context("Convenio_01102026_21102026.csv");
        let mut result = PrepareResult::default();
        result.stats.medicines = 3;
        result.stats.other_services = 2;
        result.stats.copay_lines = 1;
        result.stats.limit = 5;
        result.stats.prepared_pending = 4;
        result.stats.evaluated = 9;
        result.stats.finished = 2;
        result.stats.mipres_unavailable = 1;
        result.prepared_format = true;
        result.excluded.push(Excluded {
            invoice: "EVS2".into(),
            reason: "MIPRES ANULADO".into(),
        });
        result.manual_review.push(ManualReview {
            invoice: "EVS3".into(),
            code: "MIPRES_NO_VERIFICADO".into(),
            reason: "x".into(),
        });
        let lines = summary_lines(&SummaryInput {
            csv_name: "Convenio_01102026_21102026.csv",
            load: &load,
            nt_version: "2026-10",
            nt_entries: 7,
            lines_read: 40,
            lines_skipped: 2,
            result: &result,
        });
        assert_eq!(
            lines[0],
            "convenio: Convenio_01102026_21102026.csv (load Convenio_01102026_21102026, 2026-10-01..2026-10-21)"
        );
        assert_eq!(
            lines[1],
            "nota técnica: versión 2026-10 (Redis, 7 entradas cruzadas con el CSV)"
        );
        assert_eq!(
            lines[2],
            "lines read=40 skipped=2 | invoices prepared=0 excluded=1 manual_review=1 | medicines=3 other services=2 copay lines=1"
        );
        assert!(lines[3].starts_with("1 invoices held because MIPRES was not available"));
        assert!(lines[4].starts_with("prepared-format load detected"));
        assert_eq!(lines[5], "already finished (not counted in --limit): 2");
        assert_eq!(
            lines[6],
            "limit: 4/5 prepared (evaluated 9, excluded 1, manual_review 1)"
        );
        assert_eq!(lines[7], "  manual review EVS3: MIPRES_NO_VERIFICADO: x");
        assert_eq!(lines[8], "  excluded EVS2: MIPRES ANULADO");
        assert_eq!(lines.len(), 9);
    }

    #[test]
    fn a_plain_summary_has_three_lines_and_no_range_without_dates() {
        let load = load_context("Manual_abc.csv");
        let lines = summary_lines(&SummaryInput {
            csv_name: "Manual_abc.csv",
            load: &load,
            nt_version: "v",
            nt_entries: 0,
            lines_read: 0,
            lines_skipped: 0,
            result: &PrepareResult::default(),
        });
        assert_eq!(lines[0], "convenio: Manual_abc.csv (load Manual_abc)");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn the_saved_line_quotes_the_load() {
        assert_eq!(
            saved_line(2, "Convenio_1_2"),
            r#"saved 2 prepared invoice(s) of load "Convenio_1_2" in the cargue tables"#
        );
    }
}
