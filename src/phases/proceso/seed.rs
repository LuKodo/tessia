//! Step 0: which invoices enter the pipeline (Go `procesoSeed`).
//!
//! The source is a local CSV or the id of an existing cargue. When the cargue
//! is not yet prepared for its newest upload, `convenio preparar` runs once
//! ([`plan_preparation`] + [`commit_preparation`], so the nota técnica is read
//! a single time) and only the prepared invoices are carried forward; the
//! excluded and manual-review ones stay behind, recorded in the database.
//! Otherwise the stored dataset is read as it is.
//!
//! Differences from Go, forced by running on several machines:
//!
//! * a cargue that is already prepared for its newest upload is **not**
//!   prepared again (re-preparing replaces the dataset, resets what
//!   `conciliar` rebuilt and restarts the `convenio` states, which would
//!   corrupt a run another machine is in the middle of); `--repreparar`
//!   forces it;
//! * preparing is serialized across machines by the short lease
//!   `proceso_preparar:<cargue>`: a machine that finds it taken waits and then
//!   reuses what the other one prepared.
//!
//! Then the seed is restricted to `--invoices`, sorted, stripped of the
//! invoices already filed or closed (before `--limit`, so they never consume
//! it) and truncated to `--limit`.

use anyhow::{Result, anyhow, bail};

use super::config::RunConfig;
use super::events::{Event, Events};
use super::lease::with_lease;
use super::steps::label;
use super::supply::Supplier;
use crate::context::AppContext;
use crate::phases::convenio_preparar::source::{register, resolve_source};
use crate::phases::convenio_preparar::{
    Plan, PrepareRequest, commit_preparation, plan_preparation,
};

/// The invoices of a run and, when replacement applies, their supplier.
pub struct Seeded {
    pub cargue_id: String,
    pub invoices: Vec<String>,
    pub supplier: Option<Supplier>,
    /// This call prepared the cargue.
    pub prepared_now: bool,
}

pub fn seed(ctx: &AppContext, cfg: &RunConfig, events: &Events) -> Result<Seeded> {
    let cargue_id = cfg.cargue_id()?;
    let mut prepared_now = false;
    let mut supplier = None;
    let mut invoices = None;
    if needs_preparation(ctx, cfg)? {
        let name = format!("proceso_preparar:{cargue_id}");
        let what = format!("preparando el cargue {cargue_id}");
        let prepared = with_lease(
            ctx,
            &name,
            &what,
            cfg.tuning.prepare_wait,
            cfg.tuning.prepare_poll,
            // Another machine may have prepared it while we waited.
            || {
                if needs_preparation(ctx, cfg)? {
                    prepare_now(ctx, cfg, events).map(Some)
                } else {
                    Ok(None)
                }
            },
        )?;
        if let Some(plan) = prepared {
            prepared_now = true;
            invoices = Some(
                plan.result
                    .invoices
                    .iter()
                    .map(|i| i.invoice.clone())
                    .collect::<Vec<_>>(),
            );
            if cfg.limit > 0 && !cfg.no_replace {
                supplier = Supplier::new(ctx, &plan, cfg.tuning.min_supply_chunk)?;
            }
        }
    }
    let invoices = match invoices {
        Some(invoices) => invoices,
        None => stored_invoices(ctx, &cargue_id, cfg)?,
    };
    Ok(Seeded {
        invoices: narrow(ctx, cfg, invoices),
        cargue_id,
        supplier,
        prepared_now,
    })
}

/// Whether the dataset of the newest upload of the cargue is missing (or a
/// re-preparation was asked for while the convenio step is active).
fn needs_preparation(ctx: &AppContext, cfg: &RunConfig) -> Result<bool> {
    let resolved = resolve_source(ctx, &cfg.source)?;
    let id = resolved.info.id.clone();
    let newest = match resolved.cargue {
        Some(seq) => seq,
        None => register(ctx, resolved)?.cargue,
    };
    let current = ctx.dataset().current_cargue(&id)?;
    let prepared = current == Some(newest) && ctx.dataset().preparation(newest)?.is_some();
    Ok(!prepared || (cfg.reprepare && cfg.active.has("convenio")))
}

/// Step 0 with its events.
fn prepare_now(ctx: &AppContext, cfg: &RunConfig, events: &Events) -> Result<Plan> {
    let step = label("convenio");
    events.emit(Event::new("convenio_start").step(step));
    let started = std::time::Instant::now();
    let millis = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let request = PrepareRequest {
        source: cfg.source.clone(),
        max_amount: cfg.max_amount.clone(),
        only: cfg.invoices.clone(),
        limit: cfg.limit,
    };
    let plan = plan_preparation(ctx, &request, None)
        .and_then(|plan| commit_preparation(ctx, &plan).map(|()| plan));
    match plan {
        Ok(plan) => {
            events.emit(done_event(&plan, millis()));
            Ok(plan)
        }
        Err(err) => {
            events.emit(
                Event::new("convenio")
                    .step(step)
                    .status("FAILED")
                    .millis(millis())
                    .error(&format!("{err:#}")),
            );
            Err(err.context("convenio preparar"))
        }
    }
}

fn done_event(plan: &Plan, millis: u64) -> Event {
    let result = &plan.result;
    let mut reason = format!(
        "prepared={} excluded={} manual_review={} lines={} nota_items={}",
        result.invoices.len(),
        result.excluded.len(),
        result.manual_review.len(),
        plan.csv.lines.len(),
        plan.nt_entries
    );
    if result.stats.limit > 0 {
        reason.push_str(&format!(
            " evaluated={} limit={}",
            result.stats.evaluated, result.stats.limit
        ));
    }
    if result.stats.mipres_unavailable > 0 {
        reason.push_str(&format!(
            " mipres_unavailable={}",
            result.stats.mipres_unavailable
        ));
    }
    let mut event = Event::new("convenio")
        .step(label("convenio"))
        .status("OK")
        .millis(millis)
        .reason(&reason);
    event.done = result.invoices.len();
    event.excluded = result.excluded.len();
    event.manual = result.manual_review.len();
    event.evaluated = usize::try_from(result.stats.evaluated).unwrap_or_default();
    event.limit = usize::try_from(result.stats.limit).unwrap_or_default();
    event
}

/// The prepared invoices of the stored dataset (all of them: `--invoices`
/// narrows afterwards, so an unknown name is simply absent).
fn stored_invoices(ctx: &AppContext, cargue_id: &str, cfg: &RunConfig) -> Result<Vec<String>> {
    let subset = (!cfg.invoices.is_empty()).then_some(cfg.invoices.as_slice());
    let dataset = ctx
        .dataset()
        .read_current(cargue_id, subset)?
        .ok_or_else(|| {
            anyhow!("el cargue {cargue_id} no está preparado: ejecute `convenio preparar` primero")
        })?;
    if dataset.invoices.is_empty() && !cfg.invoices.is_empty() {
        bail!("ninguna de las facturas indicadas está preparada en el cargue {cargue_id}");
    }
    Ok(dataset.invoices.into_iter().map(|i| i.invoice).collect())
}

/// `--invoices`, sort, skip the finished ones, `--limit`.
fn narrow(ctx: &AppContext, cfg: &RunConfig, mut invoices: Vec<String>) -> Vec<String> {
    if !cfg.invoices.is_empty() {
        invoices.retain(|i| cfg.invoices.contains(i));
    }
    invoices.sort();
    invoices.dedup();
    if !invoices.is_empty() {
        match ctx.radicacion().finished(&invoices) {
            Ok(finished) if !finished.is_empty() => {
                let before = invoices.len();
                invoices.retain(|i| !finished.contains(i));
                let skipped = before - invoices.len();
                if skipped > 0 {
                    eprintln!(
                        "proceso: {skipped} invoice(s) already finished skipped (not reprocessed)"
                    );
                }
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(error = %err, "could not look up finished invoices; continuing without skipping them")
            }
        }
    }
    if cfg.limit > 0 && invoices.len() > cfg.limit {
        invoices.truncate(cfg.limit);
    }
    invoices
}
