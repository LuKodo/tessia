//! Replacement of invoices that stop before radicación (Go
//! `replacementSupplier`).
//!
//! The pool is the pending CSV invoices the initial `--limit` preparation did
//! not reach ([`Plan::result`]`.remaining`, CSV order). Each round prepares
//! at least [`MIN_SUPPLY_CHUNK`](super::config::MIN_SUPPLY_CHUNK) candidates
//! with exactly the rules of the initial preparation (cap, automatic
//! subplans, nota técnica of the pinned version, MIPRES with one
//! authenticated client), stops as soon as enough are prepared and stores the
//! partial result through [`commit_preparation`], which merges it into the
//! stored dataset and records the excluded / manual-review candidates like
//! step 0. Candidates the round did not reach go back to the front of the pool.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};

use super::events::{Event, Events};
use crate::context::AppContext;
use crate::domain::dataset::Line;
use crate::phases::convenio_preparar::catalog::Catalog;
use crate::phases::convenio_preparar::mipres::{MipresChecker, MipresError, needs_mipres};
use crate::phases::convenio_preparar::mipres_client::{MipresClient, MipresOptions};
use crate::phases::convenio_preparar::nt_catalog::{distinct_cums, load_catalog};
use crate::phases::convenio_preparar::prepare::{PrepareOptions, PrepareResult, prepare};
use crate::phases::convenio_preparar::subplan::AutomaticSubplans;
use crate::phases::convenio_preparar::{Plan, commit_preparation};

struct Pool {
    remaining: Vec<String>,
    mipres: Option<MipresClient>,
}

/// Prepares replacement invoices on demand; `supply` calls are serialized.
pub struct Supplier {
    plan: Plan,
    catalog: Catalog,
    automatic: AutomaticSubplans,
    by_invoice: HashMap<String, Vec<Line>>,
    min_chunk: usize,
    pool: Mutex<Pool>,
}

impl Supplier {
    /// The supplier of `plan`'s leftover pool, or `None` when it is empty.
    pub fn new(ctx: &AppContext, plan: &Plan, min_chunk: usize) -> Result<Option<Self>> {
        if plan.result.remaining.is_empty() {
            return Ok(None);
        }
        let mut by_invoice: HashMap<String, Vec<Line>> = HashMap::new();
        let pool: std::collections::HashSet<&String> = plan.result.remaining.iter().collect();
        for line in &plan.csv.lines {
            if pool.contains(&line.invoice) {
                by_invoice
                    .entry(line.invoice.clone())
                    .or_default()
                    .push(line.clone());
            }
        }
        let pool_lines: Vec<Line> = by_invoice.values().flatten().cloned().collect();
        let catalog = load_catalog(
            ctx.nt.as_ref(),
            &plan.nt_version,
            distinct_cums(&pool_lines),
        )
        .context("replacement: nota técnica")?;
        Ok(Some(Self {
            catalog,
            automatic: AutomaticSubplans::for_origin(plan.load.origin, &plan.automatic_subplans),
            by_invoice,
            min_chunk: min_chunk.max(1),
            pool: Mutex::new(Pool {
                remaining: plan.result.remaining.clone(),
                mipres: None,
            }),
            plan: plan.clone(),
        }))
    }

    /// Candidates still waiting.
    pub fn pool_size(&self) -> usize {
        self.lock().remaining.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Pool> {
        self.pool.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Up to `n` newly prepared (and stored) invoices in pool order; fewer
    /// than `n` means the pool ran dry. A systemic error (MIPRES abort, a
    /// failed commit) is returned.
    pub fn supply(&self, ctx: &AppContext, events: &Events, n: usize) -> Result<Vec<String>> {
        let mut pool = self.lock();
        let mut got: Vec<String> = Vec::new();
        while got.len() < n && !pool.remaining.is_empty() {
            if crate::shutdown::requested() {
                return Err(anyhow!("interrumpido"));
            }
            let want = n - got.len();
            let size = want.max(self.min_chunk).min(pool.remaining.len());
            let chunk: Vec<String> = pool.remaining.drain(..size).collect();
            got.extend(self.prepare_chunk(ctx, events, &mut pool, chunk, want)?);
        }
        got.truncate(n);
        Ok(got)
    }

    /// Evaluates `chunk` until `want` are prepared, stores the partial result
    /// and puts the unreached candidates back at the front.
    fn prepare_chunk(
        &self,
        ctx: &AppContext,
        events: &Events,
        pool: &mut Pool,
        chunk: Vec<String>,
        want: usize,
    ) -> Result<Vec<String>> {
        let lines: Vec<Line> = chunk
            .iter()
            .flat_map(|inv| self.by_invoice.get(inv).cloned().unwrap_or_default())
            .collect();
        // An explicit --invoices list stays forced past the subplan allowlist.
        let only: Vec<String> = if self.plan.only.is_empty() {
            Vec::new()
        } else {
            chunk.clone()
        };
        let finished = ctx.radicacion().finished(&chunk).unwrap_or_else(|err| {
            tracing::warn!(error = %err, "could not look up finished invoices for the replacement");
            Default::default()
        });
        if pool.mipres.is_none() && needs_mipres(&lines, &only, &self.automatic) {
            pool.mipres = Some(authenticated_client(ctx)?);
        }
        let checker: Option<&dyn MipresChecker> = pool.mipres.as_ref().map(|c| c as _);
        let result = prepare(
            &lines,
            &self.catalog,
            &PrepareOptions {
                max: self.plan.max,
                only: &only,
                limit: want,
                finished: &finished,
                automatic: &self.automatic,
                mipres: checker,
            },
        );
        let result = match result {
            Ok(result) => result,
            Err(err) => {
                // Nothing was committed: the candidates go back.
                pool.remaining.splice(0..0, chunk);
                return Err(anyhow!("preparing replacement invoices: {err}"));
            }
        };
        pool.remaining
            .splice(0..0, result.remaining.iter().cloned());
        if result.invoices.is_empty()
            && result.excluded.is_empty()
            && result.manual_review.is_empty()
        {
            return Ok(Vec::new());
        }
        self.commit(ctx, &chunk, &result)
            .context("committing replacement invoices")?;
        let mut more = Event::new("convenio_more");
        more.done = result.invoices.len();
        more.excluded = result.excluded.len();
        more.manual = result.manual_review.len();
        more.evaluated = usize::try_from(result.stats.evaluated).unwrap_or_default();
        events.emit(more);
        Ok(result.invoices.iter().map(|i| i.invoice.clone()).collect())
    }

    /// Stores the partial result as a merge of exactly `chunk`.
    fn commit(&self, ctx: &AppContext, chunk: &[String], result: &PrepareResult) -> Result<()> {
        let partial = Plan {
            only: chunk.to_vec(),
            result: result.clone(),
            ..self.plan.clone()
        };
        commit_preparation(ctx, &partial)
    }
}

fn authenticated_client(ctx: &AppContext) -> Result<MipresClient> {
    let client =
        MipresClient::new(MipresOptions::from_settings(&ctx.settings)?).context("mipres client")?;
    match client.authenticate() {
        Ok(()) => {}
        Err(MipresError::Unavailable(detail)) => {
            tracing::warn!(error = %detail, "MIPRES is not available; EVENTO NO PBS candidates will be held");
        }
        Err(other) => return Err(anyhow!(other)),
    }
    Ok(client)
}
