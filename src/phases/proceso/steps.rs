//! The steps of `proceso run`: canonical order, labels, default worker pools
//! and the `--from/--to/--skip` selection (Go `procesoOrder`,
//! `procesoMainline`, `procesoLabels`, `selectProcesoPhases`).

use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};

/// Canonical order used by `--from/--to/--skip`.
pub const ORDER: [&str; 9] = [
    "convenio",
    "thefactory",
    "fev",
    "conciliar",
    "derechos",
    "rips",
    "cuv",
    "soporte",
    "radicacion",
];

/// The per-invoice phases an invoice flows through, in order. `conciliar` is
/// a side phase reached only from `fev`; radicación is the batch gate.
pub const MAINLINE: [&str; 6] = ["thefactory", "fev", "derechos", "rips", "cuv", "soporte"];

/// Phases that have a `--workers-<phase>` pool.
pub const POOLED: [&str; 7] = [
    "thefactory",
    "fev",
    "conciliar",
    "derechos",
    "rips",
    "cuv",
    "soporte",
];

/// Event label of a step (Go `procesoLabels`).
pub fn label(step: &str) -> &'static str {
    match step {
        "convenio" => "convenio preparar",
        "derechos" => "derechos validar",
        "thefactory" => "thefactory descargar",
        "fev" => "fev verificar",
        "conciliar" => "conciliar",
        "rips" => "rips generar",
        "cuv" => "cuv generar",
        "soporte" => "soporte generar",
        "radicacion" => "radicacion run",
        _ => "",
    }
}

/// The `--workers-<phase>` defaults of Go (`procesoDefaultWorkers`).
pub fn default_workers(step: &str) -> usize {
    match step {
        "derechos" | "thefactory" => 30,
        "cuv" => 40,
        "soporte" => 20,
        _ => 8,
    }
}

/// The steps a run executes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Active(BTreeSet<&'static str>);

impl Active {
    pub fn all() -> Self {
        Self(ORDER.iter().copied().collect())
    }

    pub fn has(&self, step: &str) -> bool {
        self.0.contains(step)
    }

    /// The mainline phases that are active, in order.
    pub fn mainline(&self) -> Vec<&'static str> {
        MAINLINE.iter().copied().filter(|s| self.has(s)).collect()
    }

    /// The labels of the active steps in canonical order.
    pub fn labels(&self) -> Vec<&'static str> {
        ORDER
            .iter()
            .copied()
            .filter(|s| self.has(s))
            .map(label)
            .collect()
    }
}

fn index_of(name: &str) -> Result<usize> {
    ORDER
        .iter()
        .position(|s| *s == name)
        .ok_or_else(|| anyhow!("unknown step {name:?} (available: {})", ORDER.join(", ")))
}

/// Resolves `--from/--to/--skip` into the active steps.
pub fn select(from: Option<&str>, to: Option<&str>, skip: &[String]) -> Result<Active> {
    fn given(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|v| !v.is_empty())
    }
    let start = given(from).map(index_of).transpose()?.unwrap_or(0);
    let end = given(to)
        .map(index_of)
        .transpose()?
        .unwrap_or(ORDER.len() - 1);
    if start > end {
        bail!(
            "--from {:?} is after --to {:?}",
            from.unwrap_or_default(),
            to.unwrap_or_default()
        );
    }
    let mut skipped = BTreeSet::new();
    for name in skip.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        skipped.insert(ORDER[index_of(name)?]);
    }
    let active: BTreeSet<&'static str> = ORDER[start..=end]
        .iter()
        .copied()
        .filter(|s| !skipped.contains(s))
        .collect();
    if active.is_empty() {
        bail!("no steps selected");
    }
    Ok(Active(active))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(active: &Active) -> Vec<&'static str> {
        ORDER.iter().copied().filter(|s| active.has(s)).collect()
    }

    #[test]
    fn no_flags_selects_everything() {
        assert_eq!(names(&select(None, None, &[]).expect("all")), ORDER);
    }

    #[test]
    fn from_to_and_skip_pick_a_subset_of_the_canonical_order() {
        let active = select(Some("fev"), Some("rips"), &["conciliar".to_owned()]).expect("subset");
        assert_eq!(names(&active), ["fev", "derechos", "rips"]);
        assert_eq!(active.mainline(), ["fev", "derechos", "rips"]);
    }

    #[test]
    fn invalid_selections_are_errors() {
        let unknown = select(Some("nope"), None, &[]).expect_err("unknown");
        assert!(unknown.to_string().starts_with("unknown step \"nope\""));
        let reversed = select(Some("cuv"), Some("fev"), &[]).expect_err("reversed");
        assert_eq!(reversed.to_string(), "--from \"cuv\" is after --to \"fev\"");
        let none =
            select(Some("convenio"), Some("convenio"), &["convenio".to_owned()]).expect_err("none");
        assert_eq!(none.to_string(), "no steps selected");
        assert!(select(None, None, &["x".to_owned()]).is_err());
    }

    #[test]
    fn defaults_and_labels_are_the_go_ones() {
        let pools: Vec<(&str, usize)> = POOLED.iter().map(|s| (*s, default_workers(s))).collect();
        assert_eq!(
            pools,
            [
                ("thefactory", 30),
                ("fev", 8),
                ("conciliar", 8),
                ("derechos", 30),
                ("rips", 8),
                ("cuv", 40),
                ("soporte", 20)
            ]
        );
        assert_eq!(label("radicacion"), "radicacion run");
        assert_eq!(label("fev"), "fev verificar");
    }
}
