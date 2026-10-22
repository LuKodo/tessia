//! The claim / work / record loop of `thefactory descargar`.
//!
//! 1. plan every invoice from the artifact index (what is already there);
//! 2. claim all of them in `proceso_estado` (`claim_available`: the invoices
//!    another machine owns are skipped, so N machines split the work);
//! 3. invoices that are already complete are recorded `DONE` (`YA_EXISTE`),
//!    the others go through a worker pool (default 30, max 30) and are recorded
//!    as soon as each one finishes: `DONE` when XML, PDF and ZIP are present
//!    (the motivo is the CPScan warning, if any), `FAILED` otherwise (the
//!    motivo is the status line).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, anyhow, bail};

use super::invoice::{Env, Processed, process};
use super::plan::{Plan, TfInput, build};
use super::runtime::Runtime;
use crate::context::AppContext;
use crate::domain::{Outcome, Phase, PhaseState, valid_invoice};
use crate::phases::common::PhaseReport;
use crate::shutdown;
use crate::stores::proceso::{ClaimItem, ProcesoState};
use crate::util::par;

/// Default and maximum number of invoices in flight.
pub const DEFAULT_WORKERS: usize = 30;

const PHASE: Phase = Phase::TheFactory;

/// What a worker did with its claim.
enum Worked {
    Stored {
        complete: bool,
    },
    /// The claim was taken over (stale TTL): the result was dropped.
    Lost,
    /// Not attempted (interrupted or aborted): the claim was given back.
    Released,
}

/// Runs the phase for `inputs` of the cargue. `runtime` replaces the clients
/// built from the settings (tests); it is only built when there is work.
pub fn run_inputs(
    ctx: &AppContext,
    cargue_id: &str,
    inputs: Vec<TfInput>,
    workers: usize,
    runtime: Option<Runtime>,
) -> Result<PhaseReport> {
    let inputs = checked(inputs)?;
    let artifacts = ctx.artifacts();
    let plans = build(&artifacts, cargue_id, inputs, workers)?;
    for plan in plans.iter().filter(|p| !p.complete()) {
        println!("  {}", pending_line(plan));
    }
    let pending_count = plans.iter().filter(|p| !p.complete()).count();
    println!(
        "thefactory: pending={pending_count} complete={}",
        plans.len() - pending_count
    );
    let runtime = match runtime {
        Some(runtime) => Some(runtime),
        None if pending_count > 0 => Some(Runtime::from_settings(ctx)?),
        None => None,
    };

    let store = ctx.proceso();
    let invoices: Vec<String> = plans.iter().map(|p| p.invoice.clone()).collect();
    let previous = store.load(PHASE, Some(&invoices))?;
    let items: Vec<ClaimItem> = invoices
        .iter()
        .map(|i| ClaimItem::from_loaded(i, previous.get(i)))
        .collect();
    let claims = store.claim_available(PHASE, &items, &ctx.claim_options(cargue_id))?;

    let mut report = PhaseReport::default();
    let (mut mine, mut complete) = (Vec::new(), Vec::new());
    for plan in plans {
        if !claims.contains_key(&plan.invoice) {
            println!(
                "warning: {} is already in execution for phase {PHASE}; skipping it",
                plan.invoice
            );
            report.skip(&plan.invoice, "EN_PROCESO_OTRA_MAQUINA");
        } else if plan.complete() {
            println!("  {} YA_EXISTE", plan.invoice);
            complete.push(plan);
        } else {
            mine.push(plan);
        }
    }
    for plan in &complete {
        let outcome = Outcome::new(PhaseState::Done, plan.stored_warning.clone());
        store_outcome(ctx, &claims, plan, &outcome, &mut report)?;
    }
    let ran = work(ctx, cargue_id, mine, workers, runtime, &claims, &previous);
    let summary = ran.merge_into(&mut report);
    println!(
        "thefactory: complete={} incomplete={}",
        summary.complete, summary.incomplete
    );
    match ran.error {
        Some(err) => Err(err),
        None => Ok(report),
    }
}

/// Validates and de-duplicates the inputs.
fn checked(inputs: Vec<TfInput>) -> Result<Vec<TfInput>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for input in inputs {
        if !valid_invoice(&input.invoice) {
            bail!("factura inválida {:?}", input.invoice);
        }
        if seen.insert(input.invoice.clone()) {
            out.push(input);
        }
    }
    Ok(out)
}

/// `FEV1 pendiente (xml=false pdf=true zip=false cpscan=false)`; the `cpscan`
/// part only when the invoice has an SSC.
fn pending_line(plan: &Plan) -> String {
    let base = format!(
        "{} pendiente (xml={} pdf={} zip={}",
        plan.invoice, plan.has_xml, plan.has_pdf, plan.has_zip
    );
    if plan.ssc.is_some() {
        format!("{base} cpscan={})", plan.has_cpscan)
    } else {
        format!("{base})")
    }
}

/// Records the outcome of a claimed invoice (a lost claim is not an error).
fn store_outcome(
    ctx: &AppContext,
    claims: &HashMap<String, i64>,
    plan: &Plan,
    outcome: &Outcome,
    report: &mut PhaseReport,
) -> Result<()> {
    let version = claims.get(&plan.invoice).copied().unwrap_or_default();
    if ctx
        .proceso()
        .save_outcome(PHASE, &plan.invoice, outcome, version)?
    {
        report.push(&plan.invoice, outcome.clone());
    } else {
        report.skip(&plan.invoice, "EN_PROCESO_OTRA_MAQUINA");
    }
    Ok(())
}

/// The worker pool's results.
struct Ran {
    results: Vec<(String, Worked, Option<Outcome>)>,
    error: Option<anyhow::Error>,
}

struct Summary {
    complete: usize,
    incomplete: usize,
}

impl Ran {
    fn merge_into(&self, report: &mut PhaseReport) -> Summary {
        let mut summary = Summary {
            complete: 0,
            incomplete: 0,
        };
        for (invoice, worked, outcome) in &self.results {
            match (worked, outcome) {
                (Worked::Stored { complete }, Some(outcome)) => {
                    report.push(invoice, outcome.clone());
                    if *complete {
                        summary.complete += 1;
                    } else {
                        summary.incomplete += 1;
                    }
                }
                (Worked::Lost, _) => report.skip(invoice, "EN_PROCESO_OTRA_MAQUINA"),
                _ => {}
            }
        }
        summary
    }
}

fn work(
    ctx: &AppContext,
    cargue_id: &str,
    mine: Vec<Plan>,
    workers: usize,
    runtime: Option<Runtime>,
    claims: &HashMap<String, i64>,
    previous: &HashMap<String, ProcesoState>,
) -> Ran {
    let Some(runtime) = runtime.filter(|_| !mine.is_empty()) else {
        return Ran {
            results: Vec::new(),
            error: None,
        };
    };
    let artifacts = ctx.artifacts();
    let env = Env {
        ctx,
        artifacts: &artifacts,
        cargue_id,
        client: &runtime.thefactory,
        acquirer: &runtime.cpscan,
    };
    let abort = AtomicBool::new(false);
    let first_error: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    let results = par::map(mine, workers, |plan| {
        let run = if abort.load(Ordering::SeqCst) || shutdown::requested() {
            release(ctx, claims, previous, &plan).map(|()| (Worked::Released, None))
        } else {
            work_one(&env, claims, &plan)
        };
        match run {
            Ok((worked, outcome)) => (plan.invoice, worked, outcome),
            Err(err) => {
                abort.store(true, Ordering::SeqCst);
                let _ = release(ctx, claims, previous, &plan);
                if let Ok(mut slot) = first_error.lock() {
                    slot.get_or_insert(err);
                }
                (plan.invoice, Worked::Released, None)
            }
        }
    });
    let error = first_error
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ran { results, error }
}

fn work_one(
    env: &Env<'_>,
    claims: &HashMap<String, i64>,
    plan: &Plan,
) -> Result<(Worked, Option<Outcome>)> {
    let processed = process(env, plan)?;
    println!("  {}", processed.line);
    let outcome = outcome_of(plan, &processed);
    let version = claims.get(&plan.invoice).copied().unwrap_or_default();
    let stored = env
        .ctx
        .proceso()
        .save_outcome(PHASE, &plan.invoice, &outcome, version)?;
    if stored {
        Ok((
            Worked::Stored {
                complete: processed.complete,
            },
            Some(outcome),
        ))
    } else {
        tracing::warn!(invoice = %plan.invoice, "claim taken over; result dropped");
        Ok((Worked::Lost, None))
    }
}

/// `DONE` with the CPScan warning as motivo when XML, PDF and ZIP are present,
/// else `FAILED` with the status line (without the leading invoice). A settled
/// "CPScan has nothing" lookup of an earlier run keeps its warning.
fn outcome_of(plan: &Plan, processed: &Processed) -> Outcome {
    if !processed.complete {
        let prefix = format!("{} ", plan.invoice);
        let line = processed
            .line
            .strip_prefix(&prefix)
            .unwrap_or(&processed.line);
        return Outcome::failed(line);
    }
    let warning = if processed.warning.is_empty() {
        plan.stored_warning.clone()
    } else {
        processed.warning.clone()
    };
    Outcome::new(PhaseState::Done, warning)
}

/// Gives the claim of an invoice back (Go `CancelStarted`).
fn release(
    ctx: &AppContext,
    claims: &HashMap<String, i64>,
    previous: &HashMap<String, ProcesoState>,
    plan: &Plan,
) -> Result<()> {
    let Some(version) = claims.get(&plan.invoice) else {
        return Ok(());
    };
    let versions = HashMap::from([(plan.invoice.clone(), *version)]);
    let old: HashMap<String, ProcesoState> = previous
        .get(&plan.invoice)
        .map(|s| (plan.invoice.clone(), s.clone()))
        .into_iter()
        .collect();
    ctx.proceso()
        .release(PHASE, &versions, &old)
        .map(|_| ())
        .map_err(|e| anyhow!("no se pudo liberar el reclamo de {}: {e}", plan.invoice))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(has: (bool, bool, bool), ssc: Option<&str>, cpscan: bool) -> Plan {
        Plan {
            invoice: "FEV1".into(),
            ssc: ssc.map(str::to_owned),
            has_xml: has.0,
            has_pdf: has.1,
            has_zip: has.2,
            has_cpscan: cpscan,
            stored_warning: String::new(),
        }
    }

    #[test]
    fn plan_lines_follow_go() {
        assert_eq!(
            pending_line(&plan((false, true, false), None, false)),
            "FEV1 pendiente (xml=false pdf=true zip=false)"
        );
        assert_eq!(
            pending_line(&plan((true, true, true), Some("9"), false)),
            "FEV1 pendiente (xml=true pdf=true zip=true cpscan=false)"
        );
    }

    #[test]
    fn complete_needs_every_part_and_the_cpscan_lookup_when_there_is_an_ssc() {
        assert!(plan((true, true, true), None, false).complete());
        assert!(!plan((true, true, true), Some("9"), false).complete());
        assert!(plan((true, true, true), Some("9"), true).complete());
        assert!(!plan((true, false, true), None, false).complete());
    }

    #[test]
    fn invoices_are_validated_and_deduplicated() {
        let ok = checked(vec![
            TfInput::new("FEV1", None),
            TfInput::new("FEV1", Some("5")),
            TfInput::new("FEV2", None),
        ])
        .expect("valid");
        assert_eq!(ok.len(), 2);
        assert_eq!(ok[0].ssc, None, "first wins");
        assert!(checked(vec![TfInput::new("bad/1", None)]).is_err());
    }

    #[test]
    fn outcome_is_done_with_the_warning_or_failed_with_the_status_line() {
        let p = plan((false, false, false), None, false);
        let failed = Processed {
            line: "FEV1 XML=NO_DISPONIBLE PDF=ERROR ZIP=FALTA (XML: HTTP 404 ; PDF: x) CPSCAN=PENDIENTE".into(),
            complete: false,
            warning: "ignored".into(),
        };
        assert_eq!(
            outcome_of(&p, &failed),
            Outcome::failed(
                "XML=NO_DISPONIBLE PDF=ERROR ZIP=FALTA (XML: HTTP 404 ; PDF: x) CPSCAN=PENDIENTE"
            )
        );
        let done = Processed {
            line: "FEV1 XML=DESCARGADO PDF=DESCARGADO ZIP=OK".into(),
            complete: true,
            warning: String::new(),
        };
        assert_eq!(outcome_of(&p, &done), Outcome::done());
        let mut q = p;
        q.stored_warning = "CPSCAN sin soporte (SSC 9, origen PC)".into();
        assert_eq!(
            outcome_of(&q, &done).motivo,
            "CPSCAN sin soporte (SSC 9, origen PC)"
        );
    }
}
