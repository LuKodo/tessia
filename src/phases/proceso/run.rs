//! `proceso run`: seed, stream, alerts (Go `runProcesoRun` + `buildPipeline`).

use std::time::Instant;

use anyhow::{Result, anyhow};

use super::classify::{Verdict, classify};
use super::config::RunConfig;
use super::events::Events;
use super::pipeline::{
    Finished, Fired, GateConfig, Hooks, PhaseSpec, PipelineConfig, run_pipeline,
};
use super::runner::PhaseRunner;
use super::seed::{Seeded, seed};
use super::steps::label;
use crate::context::AppContext;
use crate::domain::{Phase, PhaseState};
use crate::phases::common::{PhaseReport, PhaseRequest};

/// What a run did.
#[derive(Debug, Clone)]
pub struct RunSummary {
    pub cargue_id: String,
    /// Invoices that entered the pipeline at the start (before replacements).
    pub seeded: usize,
    pub prepared_now: bool,
    pub finished: Finished,
}

/// Substring of the error `radicacion::run_phase` returns while another
/// machine is filing the same cargue.
const OTHER_FILER: &str = "ya lo está radicando otra máquina";

/// Runs the whole command with the given phases, writing the stream to
/// `events`. The error is the first systemic one (plus the run-wide alerts).
pub fn execute(
    ctx: &AppContext,
    cfg: &RunConfig,
    runner: &PhaseRunner,
    events: &Events,
) -> Result<RunSummary> {
    let seeded = seed(ctx, cfg, events)?;
    let finished = stream(ctx, cfg, runner, events, &seeded);
    let seeded_count = seeded.invoices.len();
    let mut errors: Vec<String> = finished.error.iter().cloned().collect();
    if let Err(alert) = runner.finish(ctx) {
        errors.push(format!("{alert:#}"));
    }
    if crate::shutdown::requested() {
        errors.push("interrumpido".to_owned());
    }
    let summary = RunSummary {
        cargue_id: seeded.cargue_id,
        seeded: seeded_count,
        prepared_now: seeded.prepared_now,
        finished,
    };
    if errors.is_empty() {
        Ok(summary)
    } else {
        Err(anyhow!(errors.join("; ")))
    }
}

fn stream(
    ctx: &AppContext,
    cfg: &RunConfig,
    runner: &PhaseRunner,
    events: &Events,
    seeded: &Seeded,
) -> Finished {
    let pipeline = pipeline_config(cfg, seeded);
    let run = |phase: &str, invoice: &str| run_one(ctx, runner, &seeded.cargue_id, phase, invoice);
    let fire = |batch: &[String]| fire_batch(ctx, cfg, runner, &seeded.cargue_id, batch);
    let supply = |n: usize| match &seeded.supplier {
        Some(supplier) => supplier.supply(ctx, events, n),
        None => Ok(Vec::new()),
    };
    let hooks = Hooks {
        run: &run,
        fire: pipeline.gate.is_some().then_some(&fire as _),
        supply: seeded.supplier.is_some().then_some(&supply as _),
    };
    run_pipeline(&pipeline, &hooks, events)
}

/// The active phases wired in order, the gate and the replacement budget.
fn pipeline_config(cfg: &RunConfig, seeded: &Seeded) -> PipelineConfig {
    let mainline = cfg.active.mainline();
    let spec = |name: &'static str, next: Option<&'static str>| PhaseSpec {
        name,
        label: label(name),
        workers: cfg.workers_of(name),
        next,
    };
    let mut phases: Vec<PhaseSpec> = mainline
        .iter()
        .enumerate()
        .map(|(at, name)| spec(name, mainline.get(at + 1).copied()))
        .collect();
    if cfg.active.has("conciliar") {
        // conciliar hands the invoice back to fev's successor.
        let after_fev = mainline
            .iter()
            .position(|s| *s == "fev")
            .and_then(|at| mainline.get(at + 1).copied());
        phases.push(spec("conciliar", after_fev));
    }
    let gate = cfg.active.has("radicacion").then(|| GateConfig {
        threshold: cfg.batch_radicacion,
        interval: cfg.radicacion_interval,
        label: label("radicacion"),
    });
    let replacing = seeded.supplier.as_ref();
    PipelineConfig {
        entry: mainline.first().copied(),
        phases,
        invoices: seeded.invoices.clone(),
        continue_on_error: cfg.continue_on_error,
        gate,
        target: if replacing.is_some() { cfg.limit } else { 0 },
        replace_cap: replacing.map_or(0, |s| s.pool_size()),
        breaker: cfg.tuning.breaker,
    }
}

fn phase_of(step: &str) -> Option<Phase> {
    match step {
        "thefactory" => Some(Phase::TheFactory),
        "fev" => Some(Phase::FevValor),
        "conciliar" => Some(Phase::Conciliacion),
        "derechos" => Some(Phase::Derechos),
        "rips" => Some(Phase::Rips),
        "cuv" => Some(Phase::Cuv),
        "soporte" => Some(Phase::Soporte),
        _ => None,
    }
}

/// One phase for one invoice: `--invoices <invoice>` semantics (the gate of
/// the phase is bypassed, the claim still applies).
fn run_one(
    ctx: &AppContext,
    runner: &PhaseRunner,
    cargue_id: &str,
    phase: &str,
    invoice: &str,
) -> Verdict {
    let Some(run) = runner.get(phase) else {
        return Verdict::failed_with(&anyhow!("la fase {phase} no tiene ejecutor"));
    };
    let request = PhaseRequest {
        cargue_id: cargue_id.to_owned(),
        invoices: Some(vec![invoice.to_owned()]),
        workers: None,
    };
    let stored = || {
        let stored = ctx
            .proceso()
            .load(phase_of(phase)?, Some(&[invoice.to_owned()]))
            .ok()?;
        stored.get(invoice).map(|s| s.state)
    };
    classify(phase, invoice, run(ctx, &request), &|| stored())
}

/// Files one batch through `radicacion`. While another machine files the same
/// cargue the batch waits (its lease serializes uploads) instead of failing.
fn fire_batch(
    ctx: &AppContext,
    cfg: &RunConfig,
    runner: &PhaseRunner,
    cargue_id: &str,
    batch: &[String],
) -> Fired {
    let Some(run) = runner.get("radicacion") else {
        return Fired::Failed("la fase radicacion no tiene ejecutor".to_owned());
    };
    let request = PhaseRequest {
        cargue_id: cargue_id.to_owned(),
        invoices: Some(batch.to_vec()),
        workers: None,
    };
    let give_up = Instant::now() + cfg.tuning.radicacion_wait;
    loop {
        match run(ctx, &request) {
            Ok(report) => return Fired::Done(batch_summary(&report)),
            Err(err) if format!("{err:#}").contains(OTHER_FILER) => {
                let wait = cfg.tuning.radicacion_retry;
                if Instant::now() + wait > give_up || crate::shutdown::requested() {
                    return Fired::OtherMachine(format!("{err:#}"));
                }
                std::thread::sleep(wait);
            }
            Err(err) => return Fired::Failed(format!("{err:#}")),
        }
    }
}

fn batch_summary(report: &PhaseReport) -> String {
    let count = |state: PhaseState| {
        report
            .outcomes
            .iter()
            .filter(|o| o.outcome.state == state)
            .count()
    };
    format!(
        "filed={} manual={} blocked={} pending={}",
        count(PhaseState::Done),
        count(PhaseState::RevisionManual),
        count(PhaseState::Failed),
        report.skipped.len()
    )
}
