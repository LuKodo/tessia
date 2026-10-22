//! Phase names, per-invoice phase states and the gating rule between phases.

use std::fmt;
use std::str::FromStr;

/// A processing phase. The string form is the exact name Go stores in
/// `proceso_estado.fase`; never change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Phase {
    /// `convenio preparar`.
    Convenio,
    /// `derechos validar`.
    Derechos,
    /// `thefactory descargar` (XML, PDF, ZIP XML/PDF).
    TheFactory,
    /// `fev ajustar`.
    FevAjuste,
    /// `fev verificar` (XML total against the convenio).
    FevValor,
    /// `conciliar` (reconciliation of `fev_valor` differences).
    Conciliacion,
    /// `rips generar`.
    Rips,
    /// `cuv generar` / `cuv barrer`.
    Cuv,
    /// `soporte generar` (ZIP CRC).
    Soporte,
}

impl Phase {
    /// Every phase in pipeline order.
    pub const ALL: [Phase; 9] = [
        Phase::Convenio,
        Phase::Derechos,
        Phase::TheFactory,
        Phase::FevAjuste,
        Phase::FevValor,
        Phase::Conciliacion,
        Phase::Rips,
        Phase::Cuv,
        Phase::Soporte,
    ];

    /// The stored name (`"fev_ajuste"`, `"thefactory"`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Convenio => "convenio",
            Phase::Derechos => "derechos",
            Phase::TheFactory => "thefactory",
            Phase::FevAjuste => "fev_ajuste",
            Phase::FevValor => "fev_valor",
            Phase::Conciliacion => "conciliacion",
            Phase::Rips => "rips",
            Phase::Cuv => "cuv",
            Phase::Soporte => "soporte",
        }
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Phase {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Phase::ALL
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| format!("fase desconocida: {s}"))
    }
}

/// Status of a phase for one invoice (`proceso_estado.estado`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhaseState {
    /// The phase completed for the invoice.
    Done,
    /// Business rules removed the invoice; the reason says why.
    Excluded,
    /// Blocked until a person reviews it; only reprocessed when requested
    /// explicitly (`--facturas`).
    RevisionManual,
    /// A failure the next run retries; `intentos` counts the tries.
    Failed,
    /// Claimed by a worker that has not finished. Only shown on dashboards;
    /// it never gates anything and the phase's own outcome overwrites it.
    EnProgreso,
}

impl PhaseState {
    /// The stored name: `DONE`, `EXCLUDED`, `REVISION_MANUAL`, `FAILED`, `EN_PROGRESO`.
    pub fn as_str(self) -> &'static str {
        match self {
            PhaseState::Done => "DONE",
            PhaseState::Excluded => "EXCLUDED",
            PhaseState::RevisionManual => "REVISION_MANUAL",
            PhaseState::Failed => "FAILED",
            PhaseState::EnProgreso => "EN_PROGRESO",
        }
    }
}

impl fmt::Display for PhaseState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PhaseState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        [
            PhaseState::Done,
            PhaseState::Excluded,
            PhaseState::RevisionManual,
            PhaseState::Failed,
            PhaseState::EnProgreso,
        ]
        .into_iter()
        .find(|p| p.as_str() == s)
        .ok_or_else(|| format!("estado desconocido: {s}"))
    }
}

/// Result of a phase for one invoice, before it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub state: PhaseState,
    /// Spanish reason code or text exactly as Go stores it (may be empty).
    pub motivo: String,
}

impl Outcome {
    pub fn new(state: PhaseState, motivo: impl Into<String>) -> Self {
        Self {
            state,
            motivo: motivo.into(),
        }
    }

    pub fn done() -> Self {
        Self::new(PhaseState::Done, "")
    }

    pub fn failed(motivo: impl Into<String>) -> Self {
        Self::new(PhaseState::Failed, motivo)
    }

    pub fn excluded(motivo: impl Into<String>) -> Self {
        Self::new(PhaseState::Excluded, motivo)
    }

    pub fn manual_review(motivo: impl Into<String>) -> Self {
        Self::new(PhaseState::RevisionManual, motivo)
    }
}

/// Why `invoice` must be skipped by a phase that was not asked for it
/// explicitly, or `None` to process it (port of Go `proceso.Gate`).
///
/// * `own` is the invoice's state in the phase being run;
/// * `previous` is its state in the phase just before, with that phase.
///
/// Returns `"REVISION_MANUAL"` when `own` is manual review, or
/// `"<PHASE>_NO_OK"` when the earlier phase recorded anything but `DONE`.
/// An invoice with no earlier state passes, so files produced outside this
/// program keep working.
pub fn gate(own: Option<PhaseState>, previous: Option<(Phase, PhaseState)>) -> Option<String> {
    if own == Some(PhaseState::RevisionManual) {
        return Some(PhaseState::RevisionManual.as_str().to_owned());
    }
    match previous {
        Some((phase, state)) if state != PhaseState::Done => {
            Some(format!("{}_NO_OK", phase.as_str().to_uppercase()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_names_round_trip_and_match_go() {
        let names: Vec<&str> = Phase::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(
            names,
            [
                "convenio",
                "derechos",
                "thefactory",
                "fev_ajuste",
                "fev_valor",
                "conciliacion",
                "rips",
                "cuv",
                "soporte"
            ]
        );
        for phase in Phase::ALL {
            assert_eq!(phase.as_str().parse::<Phase>(), Ok(phase));
        }
        assert!("nope".parse::<Phase>().is_err());
    }

    #[test]
    fn state_names_match_go() {
        assert_eq!(PhaseState::EnProgreso.as_str(), "EN_PROGRESO");
        assert_eq!(
            "REVISION_MANUAL".parse::<PhaseState>(),
            Ok(PhaseState::RevisionManual)
        );
    }

    #[test]
    fn gate_blocks_manual_review_and_unfinished_predecessors() {
        assert_eq!(
            gate(Some(PhaseState::RevisionManual), None).as_deref(),
            Some("REVISION_MANUAL")
        );
        assert_eq!(
            gate(None, Some((Phase::FevAjuste, PhaseState::Failed))).as_deref(),
            Some("FEV_AJUSTE_NO_OK")
        );
        assert_eq!(
            gate(
                Some(PhaseState::Failed),
                Some((Phase::Convenio, PhaseState::Done))
            ),
            None
        );
        assert_eq!(gate(None, None), None);
    }
}
