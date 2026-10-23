//! Decides, for one invoice, whether it gets a RIPS JSON and builds it. Go
//! source: `internal/rips/plan.go` (`MakePlan`) and `cmd/tessia/cmd_rips.go`
//! (`ripsRecordsSkip`, `gateRipsPlan`).

use anyhow::{Context, Result};

use super::builder::{
    CODE_AFFILIATE_MISMATCH, CODE_INVALID_DATA, CODE_NO_SERVICES, CODE_RIGHTS_NOT_OK, RipsOptions,
    SkipError, build,
};
use super::derechos::DerechosResult;
use super::encode::encode;
use super::input::RipsInput;
use super::model::Rips;
use crate::domain::phase::gate;
use crate::domain::{Outcome, Phase, PhaseState};
use crate::stores::proceso::ProcesoState;

const STATUS_DONE: &str = "DONE";

/// An invoice whose RIPS JSON can be written.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadyRips {
    pub invoice: String,
    pub rips: Rips,
    /// [`encode`] of `rips`: the exact bytes of `<F>.json`.
    pub json: String,
}

impl ReadyRips {
    /// The Java output name, `<F>.json` (also the artifact path).
    pub fn file_name(&self) -> String {
        format!("{}.json", self.invoice)
    }

    /// `(medicamentos, otros_servicios)` counts of the single user.
    pub fn service_counts(&self) -> (usize, usize) {
        self.rips.usuarios.first().map_or((0, 0), |u| {
            (
                u.servicios.medicamentos.as_ref().map_or(0, Vec::len),
                u.servicios.otros_servicios.as_ref().map_or(0, Vec::len),
            )
        })
    }
}

/// An invoice that gets no JSON, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub invoice: String,
    pub code: String,
    pub detail: String,
}

/// The decision for one invoice.
#[derive(Debug, Clone, PartialEq)]
pub enum Planned {
    Ready(ReadyRips),
    Skipped(Skipped),
}

/// Builds the JSON of a prepared invoice whose rights are `derechos` (`None`:
/// `derechos validar` has no result for it):
///
/// * the rights outcome must be `DONE` with an affiliate; anything else
///   (missing, `REVISION_MANUAL`) is skipped with `DERECHOS_NO_OK`;
/// * then [`build`] and [`encode`].
pub fn plan_invoice(
    input: &RipsInput,
    derechos: Option<&DerechosResult>,
    opts: &RipsOptions,
) -> Result<Planned> {
    let skip = |code: &str, detail: String| {
        Ok(Planned::Skipped(Skipped {
            invoice: input.invoice.clone(),
            code: code.to_owned(),
            detail,
        }))
    };
    let Some(result) = derechos else {
        return skip(
            CODE_RIGHTS_NOT_OK,
            format!(
                "sin resultado en derechos/{}.json (ejecutar `facturacion derechos validar`)",
                input.invoice
            ),
        );
    };
    let Some(affiliate) = result
        .affiliate
        .as_ref()
        .filter(|_| result.status == STATUS_DONE)
    else {
        return skip(
            CODE_RIGHTS_NOT_OK,
            format!("derechos {}: {}", result.status, result.reason),
        );
    };
    match build(input, affiliate, opts)? {
        Err(SkipError { code, detail }) => skip(code, detail),
        Ok(rips) => {
            let json = encode(&rips).with_context(|| format!("rips: encode {}", input.invoice))?;
            Ok(Planned::Ready(ReadyRips {
                invoice: input.invoice.clone(),
                rips,
                json,
            }))
        }
    }
}

/// Whether a skip is a `rips` outcome (stored as `REVISION_MANUAL`).
/// `CONVENIO_NO_OK` and `DERECHOS_NO_OK` belong to the earlier phases (already
/// recorded there) and gate codes are not outcomes.
pub fn records_skip(code: &str) -> bool {
    matches!(
        code,
        CODE_NO_SERVICES | CODE_INVALID_DATA | CODE_AFFILIATE_MISMATCH
    )
}

/// The `proceso_estado` outcome of a recorded skip: `REVISION_MANUAL` with the
/// motivo `<CODE>: <detail>`.
pub fn skip_outcome(skipped: &Skipped) -> Outcome {
    Outcome::manual_review(format!("{}: {}", skipped.code, skipped.detail))
}

/// Why `rips` must not redo an invoice without `--invoices`: its own
/// `REVISION_MANUAL`, or a `fev_valor` state other than `DONE` (Java blocks the
/// JSON when the XML total differs). Returns `(code, detail)` (Go `gate`).
pub fn gate_skip(
    own: Option<&ProcesoState>,
    fev_valor: Option<&ProcesoState>,
) -> Option<(String, String)> {
    let previous = fev_valor.map(|s| (Phase::FevValor, s.state));
    let code = gate(own.map(|s| s.state), previous)?;
    let state = if code == PhaseState::RevisionManual.as_str() {
        own
    } else {
        fev_valor
    }?;
    let mut detail = format!("{} {}", state.phase, state.state);
    if !state.motivo.is_empty() {
        detail.push_str(": ");
        detail.push_str(&state.motivo);
    }
    detail.push_str(" (reprocess with --invoices)");
    Some((code, detail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phases::rips::derechos::{AffiliateRights, Rights};
    use crate::phases::rips::input::RipsLine;
    use chrono::Utc;

    fn opts() -> RipsOptions {
        RipsOptions::new("806011811", "130018500201")
    }

    fn input() -> RipsInput {
        RipsInput {
            invoice: "FVS1".into(),
            lines: vec![RipsLine {
                source_line: 1,
                doc_type: "CC".into(),
                doc_number: "900000001".into(),
                date: "2026-09-01".into(),
                quantity: "1".into(),
                unit_value: "10".into(),
                total_value: "10".into(),
                copay: "0".into(),
                product_type: "799".into(),
                ..RipsLine::default()
            }],
        }
    }

    fn done() -> DerechosResult {
        DerechosResult {
            status: "DONE".into(),
            reason: String::new(),
            affiliate: Some(AffiliateRights {
                doc_type: "CC".into(),
                doc_number: "900000001".into(),
                rights: Rights {
                    user_type: "01".into(),
                    zone: "U".into(),
                    dane: "13001".into(),
                    sex: "F".into(),
                    birth_date: "1990-01-02".into(),
                },
            }),
        }
    }

    fn state(phase: Phase, state: PhaseState, motivo: &str) -> ProcesoState {
        ProcesoState {
            invoice: "FVS1".into(),
            phase,
            state,
            motivo: motivo.into(),
            intentos: 1,
            version: 3,
            cargue_id: None,
            claimed_by: None,
            claimed_at: None,
            next_attempt_at: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn rights_must_be_done_with_an_affiliate() {
        let missing = plan_invoice(&input(), None, &opts()).expect("plan");
        let Planned::Skipped(s) = missing else {
            panic!("expected skip")
        };
        assert_eq!(s.code, "DERECHOS_NO_OK");
        assert!(s.detail.starts_with("sin resultado en derechos/FVS1.json"));

        let manual = DerechosResult {
            status: "REVISION_MANUAL".into(),
            reason: "Validador de derechos no disponible".into(),
            affiliate: None,
        };
        let Planned::Skipped(s) = plan_invoice(&input(), Some(&manual), &opts()).expect("plan")
        else {
            panic!("expected skip")
        };
        assert_eq!(s.code, "DERECHOS_NO_OK");
        assert_eq!(
            s.detail,
            "derechos REVISION_MANUAL: Validador de derechos no disponible"
        );
    }

    #[test]
    fn a_done_invoice_is_ready_with_its_file_name_and_counts() {
        let Planned::Ready(r) = plan_invoice(&input(), Some(&done()), &opts()).expect("plan")
        else {
            panic!("expected ready")
        };
        assert_eq!(r.file_name(), "FVS1.json");
        assert_eq!(r.service_counts(), (0, 1));
        assert_eq!(r.json, encode(&r.rips).expect("encode"));
        assert!(!r.json.ends_with('\n'));
    }

    #[test]
    fn build_skips_become_planned_skips() {
        let mut empty = input();
        empty.lines.clear();
        let Planned::Skipped(s) = plan_invoice(&empty, Some(&done()), &opts()).expect("plan")
        else {
            panic!("expected skip")
        };
        assert_eq!(
            (s.code.as_str(), records_skip(&s.code)),
            ("SIN_SERVICIOS", true)
        );
        assert_eq!(
            skip_outcome(&s),
            Outcome::manual_review("SIN_SERVICIOS: la factura no tiene lineas en convenio.json")
        );
    }

    #[test]
    fn only_rips_outcomes_are_recorded() {
        for code in ["SIN_SERVICIOS", "DATOS_INVALIDOS", "AFILIADO_NO_COINCIDE"] {
            assert!(records_skip(code), "{code}");
        }
        for code in [
            "CONVENIO_NO_OK",
            "DERECHOS_NO_OK",
            "REVISION_MANUAL",
            "FEV_VALOR_NO_OK",
        ] {
            assert!(!records_skip(code), "{code}");
        }
    }

    #[test]
    fn the_gate_blocks_own_manual_review_and_unfinished_fev_valor() {
        let own = state(Phase::Rips, PhaseState::RevisionManual, "SIN_SERVICIOS: x");
        let (code, detail) = gate_skip(Some(&own), None).expect("gated");
        assert_eq!(code, "REVISION_MANUAL");
        assert_eq!(
            detail,
            "rips REVISION_MANUAL: SIN_SERVICIOS: x (reprocess with --invoices)"
        );

        let fev = state(Phase::FevValor, PhaseState::Failed, "");
        let (code, detail) = gate_skip(None, Some(&fev)).expect("gated");
        assert_eq!(code, "FEV_VALOR_NO_OK");
        assert_eq!(detail, "fev_valor FAILED (reprocess with --invoices)");

        let fev_done = state(Phase::FevValor, PhaseState::Done, "");
        let own_done = state(Phase::Rips, PhaseState::Done, "");
        assert_eq!(gate_skip(Some(&own_done), Some(&fev_done)), None);
        assert_eq!(gate_skip(None, None), None);
    }
}
