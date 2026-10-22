//! The seam between the orchestrator and the phases.
//!
//! [`PhaseRunner`] maps a step name (`thefactory`, `fev`, `conciliar`,
//! `derechos`, `rips`, `cuv`, `soporte`, `radicacion`) to a function that
//! runs it for the invoices of a [`PhaseRequest`]. [`PhaseRunner::real`] fills
//! it with the real `run_phase` functions; tests fill it with fakes that
//! record their calls.
//!
//! Conventions of the real wiring (they mirror the typed errors of Go):
//!
//! * conciliar returns its [`Incomplete`](crate::phases::conciliar::plan::Incomplete)
//!   as the error, so the classifier can tell manual items from failures and a
//!   Medicar outage (all invoices share the process-wide
//!   [`Outage`](crate::phases::conciliar::medicar::Outage));
//! * derechos accumulates the affiliate statistics of every call and its
//!   authentication stop is the [`DerechosError`](crate::phases::derechos::DerechosError);
//! * cuv adds its SISPRO login failures to one shared
//!   [`TokenTally`](crate::phases::cuv::TokenTally);
//! * `finish` runs once after every outcome is stored and returns the run-wide
//!   alerts (cuv login failures, derechos validator down).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};

use super::config::RunConfig;
use crate::context::AppContext;
use crate::phases::common::{PhaseReport, PhaseRequest};
use crate::phases::conciliar::{ConciliarOptions, run_phase_detailed};
use crate::phases::cuv::{self, TokenTally};
use crate::phases::derechos::{self, Policy, Stats};
use crate::phases::rips::{self, RipsOptions};
use crate::phases::{fev, radicacion, soporte, thefactory};

/// Runs one phase for the invoices of a request.
pub type PhaseFn = Arc<dyn Fn(&AppContext, &PhaseRequest) -> Result<PhaseReport> + Send + Sync>;
/// Run-wide checks after every outcome is stored.
pub type FinishFn = Arc<dyn Fn(&AppContext) -> Result<()> + Send + Sync>;

#[derive(Clone, Default)]
pub struct PhaseRunner {
    phases: HashMap<&'static str, PhaseFn>,
    finish: Option<FinishFn>,
}

impl PhaseRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the function of `step`.
    #[must_use]
    pub fn with(
        mut self,
        step: &'static str,
        run: impl Fn(&AppContext, &PhaseRequest) -> Result<PhaseReport> + Send + Sync + 'static,
    ) -> Self {
        self.phases.insert(step, Arc::new(run));
        self
    }

    #[must_use]
    pub fn with_finish(
        mut self,
        finish: impl Fn(&AppContext) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.finish = Some(Arc::new(finish));
        self
    }

    pub fn get(&self, step: &str) -> Option<&PhaseFn> {
        self.phases.get(step)
    }

    /// Runs the run-wide checks.
    pub fn finish(&self, ctx: &AppContext) -> Result<()> {
        match &self.finish {
            Some(finish) => finish(ctx),
            None => Ok(()),
        }
    }

    /// The real phases, configured by `cfg`.
    pub fn real(cfg: &RunConfig) -> Self {
        let tally = Arc::new(TokenTally::new());
        let affiliates = Arc::new(Mutex::new(Stats::default()));
        let derechos_pool = cfg.workers_of("derechos");
        let cod_prestador = cfg.cod_prestador.clone();
        let (tally_for_cuv, affiliates_for_derechos) = (tally.clone(), affiliates.clone());
        let conciliar_options = ConciliarOptions::for_proceso();
        let (check_cuv, check_derechos) = (cfg.active.has("cuv"), cfg.active.has("derechos"));
        Self::new()
            .with("thefactory", thefactory::run_phase)
            .with("fev", fev::run_verificar_phase)
            .with("conciliar", move |ctx, req| {
                let execution = run_phase_detailed(ctx, req, &conciliar_options)?;
                match execution.incomplete() {
                    Some(incomplete) => Err(incomplete.into()),
                    None => Ok(execution.report),
                }
            })
            .with("derechos", move |ctx, req| {
                let options = derechos::RunOptions {
                    concurrency: derechos_pool,
                    ..derechos::RunOptions::default()
                };
                let execution = derechos::run_with(ctx, req, &options)?;
                affiliates_for_derechos
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .add(execution.stats);
                match execution.failure {
                    Some(failure) => Err(failure.into()),
                    None => Ok(execution.report),
                }
            })
            .with("rips", move |ctx, req| {
                run_rips(ctx, req, cod_prestador.as_deref())
            })
            .with("cuv", move |ctx, req| {
                cuv::run_phase_tallied(ctx, req, &tally_for_cuv)
            })
            .with("soporte", soporte::run_phase)
            .with("radicacion", radicacion::run_phase)
            .with_finish(move |ctx| {
                let mut alerts = Vec::new();
                if check_cuv && let Err(alert) = cuv::check_alert(ctx, &tally) {
                    alerts.push(alert.to_string());
                }
                if check_derechos {
                    let stats = *affiliates.lock().unwrap_or_else(|e| e.into_inner());
                    if let Err(alert) = derechos::check_availability(&stats, &derechos_policy(ctx))
                    {
                        alerts.push(alert.to_string());
                    }
                }
                if alerts.is_empty() {
                    Ok(())
                } else {
                    Err(anyhow!(alerts.join("; ")))
                }
            })
    }
}

/// `rips generar` for the invoices of `req`, with the `--cod-prestador` of the run.
fn run_rips(
    ctx: &AppContext,
    req: &PhaseRequest,
    cod_prestador: Option<&str>,
) -> Result<PhaseReport> {
    let options = RipsOptions::from_settings(&ctx.settings, cod_prestador)?;
    let selection = rips::selection::load(ctx, req)?;
    let execution = rips::execute(ctx, req, selection, &options)?;
    match execution.errors.first() {
        Some(first) => bail!("{first}"),
        None => Ok(execution.report),
    }
}

/// The stored validator-down alert settings; defaults when unreadable.
fn derechos_policy(ctx: &AppContext) -> Policy {
    match ctx.parametros().get() {
        Ok(Some(stored)) => Policy {
            percent: u32::try_from(stored.derechos_unavailable_percent).unwrap_or(0),
            min_sample: u32::try_from(stored.derechos_min_sample).unwrap_or(0),
        },
        _ => Policy::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_runner_has_every_step() {
        let runner = PhaseRunner::real(&RunConfig::new("C1"));
        for step in [
            "thefactory",
            "fev",
            "conciliar",
            "derechos",
            "rips",
            "cuv",
            "soporte",
            "radicacion",
        ] {
            assert!(runner.get(step).is_some(), "{step}");
        }
        assert!(runner.get("siesa").is_none(), "siesa is never run");
        assert!(runner.get("convenio").is_none());
    }
}
