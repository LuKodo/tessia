//! `derechos validar`: affiliate rights validation (régimen, zona, DANE, sexo,
//! fecha de nacimiento) with the `derechos_afiliado` cache.
//!
//! Port of Go `cmd_derechos.go` + `internal/derechos` + `derechosapi` (see
//! `docs/derechos.md` in the Go repository). The validator is mandatory:
//! every invoice ends `DONE` (rights found, complete) or `REVISION_MANUAL`
//! with one of the exact Spanish reasons in [`rights`]; prepared-format loads
//! take the rights from the file's own columns and never touch the validator.
//!
//! Pieces:
//!
//! * [`client`]: Keycloak password-grant token (cached, single-flight
//!   renewal) + FHIR `Parameters` request, document-type fallbacks, retries,
//!   adaptive limiter; one client per process ([`client::shared_client`]);
//! * [`rights`]: FHIR patient mapping and the missing-field rule;
//! * [`service`]: [`service::select`] and [`service::validate`] (cache,
//!   once-per-affiliate queries, worker pool, authentication stop);
//! * [`prepared`]: rights from the file (normalization tables, blank-fill and
//!   inconsistency rule);
//! * [`availability`]: the validator-down alert;
//! * [`result_file`]: `derechos/<invoice>.json`, published as an artifact.
//!
//! Work is split between machines through `proceso_estado` claims
//! ([`execute`]): the invoices another machine owns are reported as skipped.
//! The `rips` writer reads the result files with [`result_file::read`].
//!
//! Errors that `proceso run` must tell apart travel as [`DerechosError`]
//! inside the `anyhow::Error` (`downcast_ref`): an
//! [`DerechosError::AuthenticationStopped`] is fatal for the whole pipeline.

pub mod availability;
pub mod cli;
pub mod client;
pub mod error;
pub mod prepared;
pub mod result_file;
pub mod rights;
pub mod service;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::Utc;

pub use availability::{Policy, Stats, check_availability};
pub use cli::Args;
pub use error::DerechosError;
pub use service::{InvoiceInput, InvoiceResult, LineInput};

use crate::context::AppContext;
use crate::domain::invoice::normalize_all;
use crate::domain::{Phase, PhaseState};
use crate::phases::common::{PhaseReport, PhaseRequest};
use crate::stores::proceso::{ClaimItem, ProcesoState};
use crate::util::par;
use cli::{Command, DEFAULT_CACHE_TTL, DEFAULT_CONCURRENCY};
use client::{ClientConfig, shared_client};
use service::ValidateOptions;

/// Skip reason of an invoice another machine is working on.
pub const SKIP_OTHER_MACHINE: &str = "EN_PROCESO_OTRA_MAQUINA";

/// The dataset of the cargue as this phase reads it.
#[derive(Debug, Clone, Default)]
pub struct DatasetInput {
    pub invoices: Vec<InvoiceInput>,
    /// The load is in the legacy prepared format (some line with
    /// CLASIFICACION 714/799): rights come from the file.
    pub prepared_format: bool,
}

/// Tuning of one run.
#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    /// Parallel validator calls and result writes.
    pub concurrency: usize,
    /// Cache freshness; zero does not read the cache.
    pub cache_ttl: Duration,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            concurrency: DEFAULT_CONCURRENCY,
            cache_ttl: DEFAULT_CACHE_TTL,
        }
    }
}

/// What a run did, including the failure that is reported after saving.
#[derive(Debug, Clone, Default)]
pub struct Execution {
    /// Outcomes stored by this run and invoices skipped.
    pub report: PhaseReport,
    /// Distinct-affiliate counts (for aggregating the alert across calls).
    pub stats: Stats,
    /// Invoices selected before the claims (0: nothing pending).
    pub selected: usize,
    /// Set after the outcomes were saved: authentication stop or validator down.
    pub failure: Option<DerechosError>,
}

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    let Command::Validar(args) = args.command;
    let request = args.request()?;
    let options = RunOptions {
        concurrency: request.workers_or(DEFAULT_CONCURRENCY),
        cache_ttl: args.cache_ttl,
    };
    let execution = run_with(ctx, &request, &options)?;
    print_summary(&request.cargue_id, &execution);
    match execution.failure {
        Some(failure) => Err(failure.into()),
        None => Ok(()),
    }
}

/// Runs the phase for the invoices of `req` (called by `proceso run`). The
/// stored outcomes are in the report; a failure reported after saving (an
/// authentication stop or the validator-down alert) is the returned
/// [`DerechosError`].
pub fn run_phase(ctx: &AppContext, req: &PhaseRequest) -> Result<PhaseReport> {
    let options = RunOptions {
        concurrency: req.workers_or(DEFAULT_CONCURRENCY),
        ..RunOptions::default()
    };
    let execution = run_with(ctx, req, &options)?;
    match execution.failure {
        Some(failure) => Err(failure.into()),
        None => Ok(execution.report),
    }
}

/// [`run_phase`] with explicit options, returning the whole [`Execution`].
pub fn run_with(ctx: &AppContext, req: &PhaseRequest, options: &RunOptions) -> Result<Execution> {
    let only = req.invoices.clone().unwrap_or_default();
    let dataset = load_dataset(ctx, &req.cargue_id, &only)?;
    execute(ctx, &req.cargue_id, dataset, &only, options)
}

/// Reads the invoices (and the prepared-format flag) of the cargue.
fn load_dataset(_ctx: &AppContext, _cargue_id: &str, _only: &[String]) -> Result<DatasetInput> {
    bail!("la lectura del dataset del cargue aún no está conectada (fase B)")
}

/// Selects, claims, validates and stores the rights of `dataset` (everything
/// but reading the dataset).
pub fn execute(
    ctx: &AppContext,
    cargue_id: &str,
    dataset: DatasetInput,
    only: &[String],
    options: &RunOptions,
) -> Result<Execution> {
    let names: Vec<String> = dataset.invoices.iter().map(|i| i.invoice.clone()).collect();
    let proceso = ctx.proceso();
    let previous = proceso.load(Phase::Derechos, Some(&names))?;
    let wanted = normalize_all(only);
    let (selected, unknown) = service::select(dataset.invoices, &previous, only);
    if !unknown.is_empty() {
        tracing::warn!(invoices = %unknown.join(","), "requested invoices not in the dataset");
        if unknown.len() == wanted.len() {
            bail!("none of the --invoices is in the dataset");
        }
    }
    for invoice in &selected {
        invoice.affiliate()?; // exactly one affiliate per invoice, before any query
    }
    let mut execution = Execution {
        selected: selected.len(),
        ..Execution::default()
    };
    if selected.is_empty() {
        return Ok(execution);
    }
    let validator = if dataset.prepared_format {
        None
    } else {
        let config = ClientConfig::from_settings(&ctx.settings)?;
        Some(shared_client(&config, options.concurrency)?)
    };
    let (mine, claims) = claim(ctx, cargue_id, selected, &previous, &mut execution.report)?;
    if mine.is_empty() {
        return Ok(execution);
    }
    let validation = match &validator {
        None => prepared::from_file(&mine).map(|(results, stats)| service::Validation {
            results,
            stats,
            auth_failure: None,
        }),
        Some(client) => {
            let cache = ctx.derechos_cache();
            let validate_options = ValidateOptions {
                concurrency: options.concurrency,
                cache_ttl: options.cache_ttl,
            };
            service::validate(&mine, client, Some(&cache), &validate_options)
        }
    };
    let validation = match validation {
        Ok(validation) => validation,
        Err(err) => {
            release(ctx, &claims, &HashSet::new(), &previous);
            return Err(err.into());
        }
    };
    execution.stats = validation.stats;
    let untouched = mine.len() - validation.results.len();
    let persisted = persist(
        ctx,
        cargue_id,
        &claims,
        validation.results,
        options.concurrency,
    );
    release(ctx, &claims, &persisted.settled, &previous);
    if let Some(err) = persisted.error {
        return Err(err);
    }
    for (result, stored) in persisted.stored {
        if stored {
            execution.report.push(&result.invoice, result.outcome());
        } else {
            execution.report.skip(&result.invoice, SKIP_OTHER_MACHINE);
        }
    }
    execution.failure = match validation.auth_failure {
        Some(detail) => Some(DerechosError::AuthenticationStopped { untouched, detail }),
        None if validator.is_some() => check_availability(&execution.stats, &policy(ctx)).err(),
        None => None,
    };
    Ok(execution)
}

/// Claims the selected invoices (partial claim: what other machines have not
/// taken). Returns the claimed inputs and their claim versions; the rest are
/// reported as skipped.
fn claim(
    ctx: &AppContext,
    cargue_id: &str,
    selected: Vec<InvoiceInput>,
    previous: &HashMap<String, ProcesoState>,
    report: &mut PhaseReport,
) -> Result<(Vec<InvoiceInput>, HashMap<String, i64>)> {
    let items: Vec<ClaimItem> = selected
        .iter()
        .map(|i| ClaimItem::from_loaded(&i.invoice, previous.get(&i.invoice)))
        .collect();
    let claims =
        ctx.proceso()
            .claim_available(Phase::Derechos, &items, &ctx.claim_options(cargue_id))?;
    let mut mine = Vec::new();
    for invoice in selected {
        if claims.contains_key(&invoice.invoice) {
            mine.push(invoice);
        } else {
            report.skip(&invoice.invoice, SKIP_OTHER_MACHINE);
        }
    }
    Ok((mine, claims))
}

struct Persisted {
    /// Results with whether this run stored the outcome (false: the claim was
    /// taken over by another machine).
    stored: Vec<(InvoiceResult, bool)>,
    /// Invoices whose claim is settled (outcome saved or claim lost).
    settled: HashSet<String>,
    /// The first failure to publish a file or save an outcome.
    error: Option<anyhow::Error>,
}

/// Publishes each invoice's file, then saves its outcome (bounded by
/// `concurrency`).
fn persist(
    ctx: &AppContext,
    cargue_id: &str,
    claims: &HashMap<String, i64>,
    results: Vec<InvoiceResult>,
    concurrency: usize,
) -> Persisted {
    let artifacts = ctx.artifacts();
    let proceso = ctx.proceso();
    let now = Utc::now();
    let work = |result: InvoiceResult| -> (InvoiceResult, Result<bool>) {
        let saved = result_file::publish(&artifacts, cargue_id, &result, now).and_then(|()| {
            let version = claims.get(&result.invoice).copied().unwrap_or_default();
            Ok(proceso.save_outcome(
                Phase::Derechos,
                &result.invoice,
                &result.outcome(),
                version,
            )?)
        });
        (result, saved)
    };
    let mut persisted = Persisted {
        stored: Vec::new(),
        settled: HashSet::new(),
        error: None,
    };
    for (result, saved) in par::map(results, concurrency, work) {
        match saved {
            Ok(stored) => {
                persisted.settled.insert(result.invoice.clone());
                persisted.stored.push((result, stored));
            }
            Err(err) => {
                persisted.error.get_or_insert(err);
            }
        }
    }
    persisted
}

/// Restores the claimed invoices that got no outcome to the state they had
/// before the run (Go `CancelStarted`), so dashboards do not show them in
/// progress. Best effort: failures are only logged.
fn release(
    ctx: &AppContext,
    claims: &HashMap<String, i64>,
    settled: &HashSet<String>,
    previous: &HashMap<String, ProcesoState>,
) {
    let pending: HashMap<String, i64> = claims
        .iter()
        .filter(|(invoice, _)| !settled.contains(*invoice))
        .map(|(invoice, version)| (invoice.clone(), *version))
        .collect();
    if pending.is_empty() {
        return;
    }
    if let Err(err) = ctx.proceso().release(Phase::Derechos, &pending, previous) {
        tracing::warn!(error = %err, "could not restore the derechos states of unresolved invoices");
    }
}

/// The stored validator-down alert settings; any lookup problem falls back to
/// the defaults so the alert stays on rather than silent.
fn policy(ctx: &AppContext) -> Policy {
    match ctx.parametros().get() {
        Ok(Some(stored)) => Policy {
            percent: u32::try_from(stored.derechos_unavailable_percent).unwrap_or(0),
            min_sample: u32::try_from(stored.derechos_min_sample).unwrap_or(0),
        },
        _ => Policy::default(),
    }
}

/// The console lines of Go's `derechos validar`.
fn print_summary(cargue_id: &str, execution: &Execution) {
    if execution.selected == 0 {
        println!("no invoices pending rights validation in {cargue_id}");
        return;
    }
    let mut done = 0;
    for stored in &execution.report.outcomes {
        if stored.outcome.state == PhaseState::Done {
            done += 1;
        } else {
            println!(
                "  {} {}: {}",
                stored.invoice, stored.outcome.state, stored.outcome.motivo
            );
        }
    }
    for (invoice, reason) in &execution.report.skipped {
        println!("  {invoice} omitida: {reason}");
    }
    println!(
        "rights validated: done={done} manual_review={} (saved {}/)",
        execution.report.outcomes.len() - done,
        result_file::RESULT_DIR
    );
    if let Some(DerechosError::AuthenticationStopped { untouched, .. }) = &execution.failure {
        println!(
            "El validador de derechos rechazo el token o las credenciales aun despues de renovarlo: se detuvo la validacion. {untouched} factura(s) quedaron sin tocar y se retoman al volver a ejecutar; corrige las credenciales (derechos.*) y ejecuta de nuevo."
        );
    }
}
