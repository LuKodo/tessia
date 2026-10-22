//! Automatic-subplan allowlist and invoice selection helpers (port of the
//! subplan part of Go `convenio/prepare.go`).

use std::collections::HashSet;

use crate::domain::dataset::Line;
use crate::domain::normalize_invoice;
use crate::stores::parametros::default_automatic_subplans;
use crate::util::text::normalize_key;

/// The subplans filed automatically. Any other subplan is prepared only when
/// its invoice is named with `--invoices`. Matching is case, accent and
/// spacing insensitive.
#[derive(Debug, Clone)]
pub struct AutomaticSubplans {
    keys: HashSet<String>,
}

impl AutomaticSubplans {
    /// The hard-coded defaults (`EVENTO POS`, `EVENTO NO PBS`,
    /// `TRANSFERENCIA DE PENDIENTES CON AUTORIZACION`).
    pub fn defaults() -> Self {
        Self::from_list(&default_automatic_subplans())
    }

    /// An explicit list (the stored parameters); an empty list allows none.
    pub fn from_list(list: &[String]) -> Self {
        Self {
            keys: list.iter().map(|s| normalize_key(s)).collect(),
        }
    }

    /// True when invoices of `subplan` are prepared automatically.
    pub fn allows(&self, subplan: &str) -> bool {
        self.keys.contains(&normalize_key(subplan))
    }
}

/// The invoice's subplan: the first non-empty line value.
pub fn subplan_of<'a>(lines: impl IntoIterator<Item = &'a Line>) -> String {
    lines
        .into_iter()
        .find(|line| !line.subplan.trim().is_empty())
        .map(|line| line.subplan.clone())
        .unwrap_or_default()
}

/// Normalized, non-empty invoice names of `--invoices`.
pub fn selected_set(only: &[String]) -> HashSet<String> {
    only.iter()
        .map(|invoice| normalize_invoice(invoice))
        .filter(|invoice| !invoice.is_empty())
        .collect()
}

/// The distinct invoices (CSV order) the preparation would look at:
/// restricted to `only` when it is not empty.
pub fn candidate_invoices(lines: &[Line], only: &[String]) -> Vec<String> {
    let selected = selected_set(only);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in lines {
        if line.invoice.is_empty() || (!selected.is_empty() && !selected.contains(&line.invoice)) {
            continue;
        }
        if seen.insert(line.invoice.clone()) {
            out.push(line.invoice.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_tolerant_of_case_accents_and_spacing() {
        let auto = AutomaticSubplans::defaults();
        assert!(auto.allows("EVENTO POS"));
        assert!(auto.allows("  evento  no pbs "));
        assert!(auto.allows("Transferencia de Pendientes con Autorización"));
        assert!(!auto.allows("EVENTO SAS SIN AUTORIZACION"));
        assert!(!auto.allows(""));
    }

    #[test]
    fn an_explicit_list_replaces_the_defaults_and_an_empty_one_allows_none() {
        let custom = AutomaticSubplans::from_list(&["custom plan".to_owned()]);
        assert!(custom.allows("CUSTOM PLAN"));
        assert!(!custom.allows("EVENTO POS"));
        assert!(!AutomaticSubplans::from_list(&[]).allows("EVENTO POS"));
    }

    #[test]
    fn the_subplan_is_the_first_non_empty_line_value() {
        let lines = [
            Line::default(),
            Line {
                subplan: "A".into(),
                ..Line::default()
            },
            Line {
                subplan: "B".into(),
                ..Line::default()
            },
        ];
        assert_eq!(subplan_of(&lines), "A");
        assert_eq!(subplan_of(&lines[..1]), "");
    }

    #[test]
    fn candidates_keep_csv_order_and_honor_only() {
        let line = |invoice: &str| Line {
            invoice: invoice.into(),
            ..Line::default()
        };
        let lines = [
            line("EVS1"),
            line(""),
            line("EVS2"),
            line("EVS1"),
            line("EVS3"),
        ];
        assert_eq!(candidate_invoices(&lines, &[]), ["EVS1", "EVS2", "EVS3"]);
        assert_eq!(
            candidate_invoices(&lines, &["evs-3".to_owned(), "EVS1".to_owned()]),
            ["EVS1", "EVS3"]
        );
    }
}
