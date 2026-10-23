//! `radicacion run` and `radicacion recheck-step2`: the Mutual Ser 4-step filing pipeline.
//!
//! Port of Go `cmd_radicacion.go` + `internal/radicacion` (+ the Mutual Ser
//! adapter). Layout of this module:
//!
//! * [`rules`] / [`confirm`]: the pure rules and the four-step confirmation;
//! * [`client`] ([`api::RadApi`]): the Mutual Ser HTTP client;
//! * [`engine`]: the per-invoice pipeline (stages, micro-batches, retries);
//! * [`files`] / [`archive`]: the cargue's artifacts as the step files, and
//!   the "filed" marker that replaces moving files into `Radicado/`;
//! * [`values`]: the step 1 value check inputs;
//! * [`recheck`], [`timings`]: `recheck-step2` and the "Tiempos de radicación" report.
//!
//! One machine files a cargue at a time: [`run_cargue`] holds the lease
//! `radicacion:<cargue>` (renewed while running). `proceso run` should call
//! [`run_phase`] for one cargue sequentially.

pub mod api;
pub mod archive;
pub mod cli;
pub mod client;
pub mod confirm;
pub mod engine;
pub mod files;
pub mod model;
pub mod portal;
pub mod progress;
pub mod recheck;
pub mod rules;
pub mod timings;
pub mod values;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::context::AppContext;
use crate::domain::Outcome;
use crate::domain::radicacion::{RadStatus, Step};
use crate::phases::common::{PhaseReport, PhaseRequest};
use crate::stores::leases::LeaseGuard;
use api::RadApi;
use archive::ArchiveStore;
use client::{ClientConfig, MutualSerClient};
use engine::{Config, Deps, Engine, Report};
use files::{ArtifactFiles, Files};
use timings::{Snapshot, Timings};
use values::Values;

pub use cli::Args;

/// Artifact that keeps the last run's timings (Go `radicacion_tiempos.json`).
pub const TIMINGS_ARTIFACT: &str = "radicacion_tiempos.json";

/// Lease TTL and renewal period of the single-filer exclusion per cargue.
const LEASE_TTL: Duration = Duration::from_secs(15 * 60);
const LEASE_RENEW_EVERY: Duration = Duration::from_secs(5 * 60);

/// What a run needs besides the context.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Engine tuning (the NIT is filled in from the settings).
    pub engine: Config,
    /// Follow the portal result of each RIPS JSON upload in the background.
    pub portal_follow: bool,
    /// Waits between portal queries of one invoice (empty = 10 s, 15 s, 30 s, then 60 s x 9).
    pub portal_schedule: Vec<Duration>,
}

/// What [`run_cargue`] did.
pub struct RunOutcome {
    pub report: Report,
    pub timings: Snapshot,
}

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    match args.command {
        cli::Command::Run(args) => run_command(ctx, &args),
        cli::Command::RecheckStep2(args) => recheck_command(ctx, &args),
    }
}

fn run_command(ctx: &AppContext, args: &cli::RunArgs) -> Result<()> {
    let cargue_id = args.cargue_id()?;
    let invoices = normalized(&args.invoices);
    let opts = RunOptions {
        engine: engine_config(args),
        portal_follow: args.portal_follow,
        portal_schedule: Vec::new(),
    };
    let requested = (!invoices.is_empty()).then_some(invoices.as_slice());
    let Some(outcome) = run_cargue(ctx, cargue_id, requested, &opts)? else {
        println!("no se encontraron facturas en el cargue {cargue_id}");
        return Ok(());
    };
    print_report(&outcome.report);
    println!("\n{}", outcome.timings.text());
    println!("Detalle de tiempos: {TIMINGS_ARTIFACT} (artefacto del cargue {cargue_id})");
    if crate::shutdown::requested() {
        bail!("interrumpido");
    }
    Ok(())
}

fn engine_config(args: &cli::RunArgs) -> Config {
    let positive =
        |d: Option<cli::SignedDuration>| d.map(cli::SignedDuration::positive).unwrap_or_default();
    Config {
        batch_max: args.batch_max.unwrap_or(0),
        prepare_concurrency: args.prepare_concurrency.unwrap_or(0),
        precheck_timeout: args
            .precheck_timeout
            .map(cli::SignedDuration::wait)
            .unwrap_or_default(),
        deadline: positive(args.deadline),
        retry_interval: positive(args.retry_interval),
        reload_wait: args
            .reload_wait
            .map(cli::SignedDuration::wait)
            .unwrap_or_default(),
        max_sends: args.max_sends.unwrap_or(0),
        only_step: args.step.and_then(|n| Step::from_i16(i16::from(n))),
        ..Config::default()
    }
}

fn recheck_command(ctx: &AppContext, args: &cli::RecheckStep2Args) -> Result<()> {
    let invoices = normalized(&args.invoices);
    if invoices.is_empty() {
        bail!("uso: facturacion radicacion recheck-step2 --invoices a,b");
    }
    let nit = nit(ctx)?;
    let api = build_client(ctx, &nit, None)?;
    let report = recheck::recheck_step2(&api, &ctx.radicacion(), &invoices)?;
    println!("{}", report.line());
    if report.interrupted {
        bail!("interrumpido");
    }
    Ok(())
}

/// Runs the phase for the invoices of `req` (called by `proceso run`) with the
/// default tuning. See [`run_phase_with`].
pub fn run_phase(ctx: &AppContext, req: &PhaseRequest) -> Result<PhaseReport> {
    run_phase_with(ctx, req, &RunOptions::default())
}

/// [`run_phase`] with explicit options. The timings block goes to stderr (the
/// stdout of `proceso run` is its own stream); `radicacion_tiempos.json` keeps
/// the last batch.
pub fn run_phase_with(
    ctx: &AppContext,
    req: &PhaseRequest,
    opts: &RunOptions,
) -> Result<PhaseReport> {
    let outcome = run_cargue(ctx, &req.cargue_id, req.invoices.as_deref(), opts)?;
    let Some(outcome) = outcome else {
        return Ok(PhaseReport::default());
    };
    eprintln!("\n{}", outcome.timings.text());
    eprintln!(
        "Detalle de tiempos: {TIMINGS_ARTIFACT} (artefacto del cargue {})",
        req.cargue_id
    );
    Ok(phase_report(&outcome.report))
}

/// The engine report as a phase report: filed (or advanced) = DONE; notes and
/// exhausted retries = REVISION_MANUAL with the status as motivo; blocked =
/// FAILED with the block code; still running at the deadline = skipped.
pub fn phase_report(report: &Report) -> PhaseReport {
    let mut out = PhaseReport::default();
    for invoice in report.done.iter().chain(&report.advanced) {
        out.push(invoice, Outcome::done());
    }
    for invoice in &report.rejected {
        let status = report
            .states
            .get(invoice)
            .map_or(RadStatus::Rejected, |s| s.status);
        out.push(invoice, Outcome::manual_review(status.as_str()));
    }
    for invoice in &report.blocked {
        let detail = report
            .states
            .get(invoice)
            .map(|s| s.last_detail.as_str())
            .unwrap_or_default();
        out.push(invoice, Outcome::failed(block_code(detail)));
    }
    for invoice in &report.in_progress {
        out.skip(invoice, "RADICACION_EN_PROGRESO");
    }
    for invoice in &report.unclassified {
        out.skip(invoice, "RADICACION_PENDIENTE");
    }
    out
}

/// The `codigo=` token of a value-check block, else `BLOQUEADA`.
fn block_code(detail: &str) -> String {
    detail
        .strip_prefix("codigo=")
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("BLOQUEADA")
        .to_owned()
}

fn normalized(invoices: &[String]) -> Vec<String> {
    crate::domain::invoice::normalize_all(
        invoices.iter().map(|i| crate::domain::normalize_invoice(i)),
    )
}

/// The issuer NIT: `zonaser.organization_nit`, else `company.nit_emisor`.
fn nit(ctx: &AppContext) -> Result<String> {
    let settings = &ctx.settings;
    let own = settings
        .get("zonaser.organization_nit")
        .map(str::trim)
        .unwrap_or_default();
    let nit = if own.is_empty() {
        settings
            .get("company.nit_emisor")
            .map(str::trim)
            .unwrap_or_default()
    } else {
        own
    };
    if nit.is_empty() {
        bail!("falta el NIT: defina zonaser.organization_nit o company.nit_emisor");
    }
    Ok(nit.to_owned())
}

/// Builds the Mutual Ser client from the `zonaser.*` settings, sharing the
/// process-wide `mutualser` limiter.
fn build_client(
    ctx: &AppContext,
    nit: &str,
    timings: Option<Arc<Timings>>,
) -> Result<MutualSerClient> {
    ctx.settings.require(&[
        "zonaser.login_url",
        "zonaser.benavente_base_url",
        "zonaser.username",
        "zonaser.password",
    ])?;
    let settings = &ctx.settings;
    let text = |key: &str| settings.get(key).unwrap_or_default().to_owned();
    let mut cfg = ClientConfig::new(&text("zonaser.benavente_base_url"), nit);
    cfg.login_url = text("zonaser.login_url");
    cfg.username = text("zonaser.username");
    cfg.password = text("zonaser.password");
    cfg.organization_name = text("zonaser.organization_name");
    cfg.user_email = text("zonaser.user_email");
    cfg.user_id = text("zonaser.user_id");
    cfg.portal_origin = text("zonaser.portal_origin");
    cfg.roles = settings.list("zonaser.roles");
    cfg.ca_file = settings.get("zonaser.ca_file").map(str::to_owned);
    let limiter = ctx
        .limiters
        .get("mutualser")
        .context("el limitador de Mutual Ser no está registrado")?;
    MutualSerClient::new(cfg, limiter, timings)
}

/// Keeps the cargue lease alive while a run works and frees it afterwards.
struct LeaseKeeper {
    _guard: Arc<LeaseGuard>,
    stop: Option<crossbeam_channel::Sender<()>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl LeaseKeeper {
    fn start(ctx: &AppContext, cargue_id: &str) -> Result<LeaseKeeper> {
        let name = format!("radicacion:{cargue_id}");
        let guard = ctx
            .leases()
            .guard(&name, &ctx.owner, LEASE_TTL)?
            .ok_or_else(|| {
                let holder = ctx.leases().holder(&name).ok().flatten().unwrap_or_default();
                anyhow::anyhow!(
                    "el cargue {cargue_id} ya lo está radicando otra máquina ({holder}); espere a que termine"
                )
            })?;
        let guard = Arc::new(guard);
        let (stop, stopped) = crossbeam_channel::bounded::<()>(0);
        let keeper = guard.clone();
        let handle = std::thread::spawn(move || {
            while let Err(crossbeam_channel::RecvTimeoutError::Timeout) =
                stopped.recv_timeout(LEASE_RENEW_EVERY)
            {
                match keeper.renew() {
                    Ok(true) => {}
                    Ok(false) => tracing::error!(
                        lease = keeper.name(),
                        "se perdió la exclusión de radicación del cargue"
                    ),
                    Err(err) => {
                        tracing::warn!(lease = keeper.name(), error = %err, "no se pudo renovar la exclusión de radicación")
                    }
                }
            }
        });
        Ok(LeaseKeeper {
            _guard: guard,
            stop: Some(stop),
            handle: Some(handle),
        })
    }
}

impl Drop for LeaseKeeper {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Files the invoices of a cargue in Mutual Ser. `invoices` is normalized
/// input; `None` = every invoice that has step files in the cargue. Returns
/// `None` when there is nothing to file.
pub fn run_cargue(
    ctx: &AppContext,
    cargue_id: &str,
    invoices: Option<&[String]>,
    opts: &RunOptions,
) -> Result<Option<RunOutcome>> {
    let nit = nit(ctx)?;
    let timings = Arc::new(Timings::new());
    let client = Arc::new(build_client(ctx, &nit, Some(timings.clone()))?);
    let api: Arc<dyn RadApi> = client.clone();
    let _lease = LeaseKeeper::start(ctx, cargue_id)?;
    let workdir = ctx.workdir(cargue_id)?;
    let files = ArtifactFiles::load(
        ctx.artifacts(),
        ArchiveStore::new(ctx.pool.clone()),
        workdir,
        cargue_id,
        &nit,
    )?;
    let invoices = match invoices {
        Some(list) => normalized(list),
        None => files.invoices(),
    };
    if invoices.is_empty() {
        return Ok(None);
    }
    let files: Arc<dyn Files> = Arc::new(files);
    let values = Values::from_dataset(ctx.valor_xml(), ctx.dataset());
    let mut deps = Deps::new(
        api,
        ctx.radicacion(),
        files,
        Some(Arc::new(values)),
        timings.clone(),
    );
    let follower = opts.portal_follow.then(|| {
        Arc::new(portal::Follower::start(
            client.clone(),
            portal::PortalStore::new(ctx.pool.clone()),
            opts.portal_schedule.clone(),
        ))
    });
    deps.on_uploaded = follower.clone().map(portal_hook);
    deps.on_progress = progress::lookup(cargue_id);
    let cfg = Config {
        nit,
        ..opts.engine.clone()
    };
    let report = Engine::new(cfg, deps).run(cargue_id, &invoices);
    if let Some(follower) = &follower {
        tracing::info!(
            max_s = portal::DRAIN.as_secs(),
            "waiting for the portal follow-up"
        );
        follower.close(portal::DRAIN);
    }
    let report = report?;
    let snapshot = timings.snapshot();
    snapshot.log();
    publish_timings(ctx, cargue_id, &snapshot);
    Ok(Some(RunOutcome {
        report,
        timings: snapshot,
    }))
}

/// The engine hook of the portal follow-up: only the RIPS JSON upload (step 3)
/// is followed.
fn portal_hook(follower: Arc<portal::Follower>) -> engine::UploadedHook {
    Arc::new(move |invoice, step| {
        if step == Step::Rips {
            follower.enqueue(invoice);
        }
    })
}

/// Keeps the timings as an artifact of the cargue; a failure is only logged
/// (the timings can never fail a run).
fn publish_timings(ctx: &AppContext, cargue_id: &str, snapshot: &Snapshot) {
    let result = snapshot
        .json()
        .map_err(anyhow::Error::from)
        .and_then(|bytes| {
            ctx.artifacts()
                .publish_bytes(cargue_id, TIMINGS_ARTIFACT, &bytes)
        });
    if let Err(err) = result {
        tracing::warn!(error = %err, "could not store the radicación timings");
    }
}

/// Prints the report of a run (same lines as Go).
fn print_report(report: &Report) {
    println!("{}", report.summary());
    let groups: [(&str, &Vec<String>); 4] = [
        ("rejected (notes required)", &report.rejected),
        ("blocked", &report.blocked),
        ("in progress", &report.in_progress),
        ("unclassified", &report.unclassified),
    ];
    for (label, list) in groups {
        if list.is_empty() {
            continue;
        }
        let shown = &list[..list.len().min(40)];
        println!("{label} ({}): {}", list.len(), shown.join(", "));
        for invoice in shown {
            if let Some(st) = report
                .states
                .get(invoice)
                .filter(|s| !s.last_detail.is_empty())
            {
                println!(
                    "  {invoice} paso={} {}: {}",
                    st.step.as_i16(),
                    st.status,
                    truncate_detail(&st.last_detail)
                );
            }
        }
    }
}

fn truncate_detail(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > 300 {
        format!("{}...", line.chars().take(300).collect::<String>())
    } else {
        line
    }
}

#[cfg(test)]
mod engine_tests;
