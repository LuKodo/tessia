//! `rips generar`: the RIPS JSON of each invoice (Resolución 2275), a port of
//! the Go `rips generar` (`cmd/tessia/cmd_rips.go`, `internal/rips`).
//!
//! # Flow ([`execute`])
//!
//! 1. [`selection::load`] lists the invoices of the cargue (`--invoices` limits
//!    them); convenio-excluded ones are skipped with `CONVENIO_NO_OK`.
//! 2. Without `--invoices` the gates apply: an invoice whose own `rips` state is
//!    `REVISION_MANUAL`, or whose `fev_valor` state is not `DONE`, is skipped.
//! 3. The remaining invoices are *claimed* in `proceso_estado` (so several
//!    machines split the work; the ones another machine owns are skipped).
//! 4. Worker threads read `derechos/<F>.json` from the artifacts, build and
//!    encode the document ([`plan::plan_invoice`]), publish `<F>.json` and
//!    store the outcome: `DONE`, or `REVISION_MANUAL` with `<CODE>: <detail>`
//!    for `SIN_SERVICIOS`, `DATOS_INVALIDOS` and `AFILIADO_NO_COINCIDE`.
//!    `CONVENIO_NO_OK` and `DERECHOS_NO_OK` are not recorded (the earlier phase
//!    owns that state): the claim is released.
//!
//! The Go folder lock (`.tessia-rips`) is replaced by the per-invoice claim of
//! step 3: a folder-wide lease would stop other machines from splitting the
//! work, and the claim already excludes concurrent writers of one `<F>.json`.

pub mod builder;
pub mod cli;
pub mod derechos;
pub mod encode;
pub mod input;
pub mod java;
pub mod model;
pub mod plan;
pub mod selection;

use std::collections::HashMap;

use anyhow::{Context, Result, bail};

use crate::config::Settings;
use crate::context::AppContext;
use crate::domain::{Outcome, Phase};
use crate::phases::common::{PhaseReport, PhaseRequest};
use crate::shutdown;
use crate::stores::proceso::{ClaimItem, ProcesoState};
use crate::util::par;

pub use builder::{CODE_CONVENIO_NOT_OK, HourFn, RipsOptions, deterministic_hour};
pub use cli::Args;
pub use plan::{Planned, ReadyRips, Skipped};
pub use selection::{Candidate, Selection};

/// Worker threads when the request does not say (Go wrote the files serially;
/// the work here is bucket I/O).
pub const DEFAULT_WORKERS: usize = 4;

/// Skip reason of an invoice another machine has claimed.
pub const OTHER_MACHINE: &str = "EN_PROCESO_OTRA_MAQUINA";

/// Skip reason of an invoice left unprocessed by Ctrl-C.
pub const INTERRUPTED: &str = "INTERRUMPIDO";

impl RipsOptions {
    /// `numDocumentoIdObligado` is `company.nit_emisor` (required);
    /// `codPrestador` is `cod_prestador`, else `company.codigo_prestador` +
    /// `01`, and must be 12 digits.
    pub fn from_settings(settings: &Settings, cod_prestador: Option<&str>) -> Result<Self> {
        let nit = settings.get("company.nit_emisor").unwrap_or("").trim();
        if nit.is_empty() {
            bail!("company.nit_emisor is empty");
        }
        let code = match cod_prestador.map(str::trim).filter(|c| !c.is_empty()) {
            Some(code) => code.to_owned(),
            None => format!(
                "{}01",
                settings
                    .get("company.codigo_prestador")
                    .unwrap_or("")
                    .trim()
            ),
        };
        if code.len() != 12 || !code.bytes().all(|b| b.is_ascii_digit()) {
            bail!(
                "--cod-prestador must be 12 digits (REPS code + sede), got {}",
                java::go_quote(&code)
            );
        }
        Ok(RipsOptions::new(nit, &code))
    }
}

/// What a run did, with the data of the console lines.
#[derive(Debug, Default)]
pub struct Execution {
    /// Outcomes stored and skips, as `proceso run` consumes them.
    pub report: PhaseReport,
    /// Invoices whose JSON was published: `(invoice, file, medicamentos, otros_servicios)`.
    pub written: Vec<(String, String, usize, usize)>,
    /// Every invoice that got no JSON, in the order found.
    pub skipped: Vec<Skipped>,
    /// Invoice-level failures (publishing, reading rights, storing state).
    pub errors: Vec<String>,
}

impl Execution {
    /// An invoice that was not processed (not recorded in `proceso_estado`).
    fn not_processed(&mut self, skipped: Skipped, reason: &str) {
        self.report.skip(&skipped.invoice, reason);
        self.skipped.push(skipped);
    }

    fn not_processed_by_code(&mut self, skipped: Skipped) {
        let code = skipped.code.clone();
        self.not_processed(skipped, &code);
    }
}

/// How a claimed invoice ended.
enum Finished {
    Written(ReadyRips),
    Recorded(Skipped),
    NotRecorded(Skipped),
    Lost(String),
    Interrupted(String),
}

struct Job {
    input: input::RipsInput,
    version: i64,
}

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    let cli::Command::Generar(generar) = args.command;
    let req = generar.common.to_request()?;
    let opts = RipsOptions::from_settings(&ctx.settings, generar.cod_prestador.as_deref())?;
    let selection = selection::load(ctx, &req)?;
    let exec = execute(ctx, &req, selection, &opts)?;
    print_execution(&exec, &req.cargue_id);
    match exec.errors.first() {
        Some(first) => bail!("{first}"),
        None => Ok(()),
    }
}

/// Runs the phase for the invoices of `req` (called by `proceso run`).
pub fn run_phase(ctx: &AppContext, req: &PhaseRequest) -> Result<PhaseReport> {
    let opts = RipsOptions::from_settings(&ctx.settings, None)?;
    let selection = selection::load(ctx, req)?;
    let exec = execute(ctx, req, selection, &opts)?;
    match exec.errors.first() {
        Some(first) => bail!("{first}"),
        None => Ok(exec.report),
    }
}

fn print_execution(exec: &Execution, cargue_id: &str) {
    for s in &exec.skipped {
        println!("  skip {} {}: {}", s.invoice, s.code, s.detail);
    }
    for (invoice, file, meds, oss) in &exec.written {
        println!("  {invoice} -> {file} (medicamentos={meds} otros_servicios={oss})");
    }
    println!(
        "rips: ready={} skipped={}",
        exec.written.len(),
        exec.skipped.len()
    );
    if !exec.written.is_empty() {
        println!("rips: wrote {} file(s) in {cargue_id}", exec.written.len());
    }
}

/// The whole phase over an already read [`Selection`] (see the module docs).
pub fn execute(
    ctx: &AppContext,
    req: &PhaseRequest,
    selection: Selection,
    opts: &RipsOptions,
) -> Result<Execution> {
    check_requested(req, &selection)?;
    let explicit = req.invoices.as_ref().is_some_and(|v| !v.is_empty());
    let mut exec = Execution::default();
    let mut prepared = Vec::new();
    for candidate in selection.candidates {
        match candidate {
            Candidate::ConvenioNotOk { invoice, detail } => exec.not_processed_by_code(Skipped {
                invoice,
                code: CODE_CONVENIO_NOT_OK.to_owned(),
                detail,
            }),
            Candidate::Prepared(input) => prepared.push(input),
        }
    }
    let names: Vec<String> = prepared.iter().map(|i| i.invoice.clone()).collect();
    let store = ctx.proceso();
    let own = store.load(Phase::Rips, Some(&names))?;
    let fev = if explicit {
        HashMap::new()
    } else {
        store.load(Phase::FevValor, Some(&names))?
    };
    let mut open = Vec::new();
    for input in prepared {
        match plan::gate_skip(own.get(&input.invoice), fev.get(&input.invoice)) {
            Some((code, detail)) if !explicit => exec.not_processed_by_code(Skipped {
                invoice: input.invoice,
                code,
                detail,
            }),
            _ => open.push(input),
        }
    }
    let jobs = claim(ctx, req, &own, open, &mut exec)?;
    let results = par::map(jobs, req.workers_or(DEFAULT_WORKERS), |job| {
        work(ctx, req, opts, &own, job)
    });
    for result in results {
        match result {
            Ok(finished) => fold(&mut exec, finished),
            Err(err) => exec.errors.push(format!("{err:#}")),
        }
    }
    Ok(exec)
}

/// Fails when `--invoices` named invoices and none of them is in the dataset;
/// warns about the ones that are not.
fn check_requested(req: &PhaseRequest, selection: &Selection) -> Result<()> {
    if selection.unknown.is_empty() {
        return Ok(());
    }
    tracing::warn!(
        invoices = %selection.unknown.join(","),
        "requested invoices not in the dataset"
    );
    let requested = req.invoices.as_ref().map_or(0, Vec::len);
    if requested > 0 && selection.unknown.len() == requested {
        bail!("none of the --invoices is in the dataset");
    }
    Ok(())
}

/// Claims the invoices (Go `MarkStarted`); the ones another machine owns are
/// reported as skipped.
fn claim(
    ctx: &AppContext,
    req: &PhaseRequest,
    own: &HashMap<String, ProcesoState>,
    open: Vec<input::RipsInput>,
    exec: &mut Execution,
) -> Result<Vec<Job>> {
    let items: Vec<ClaimItem> = open
        .iter()
        .map(|i| ClaimItem::from_loaded(&i.invoice, own.get(&i.invoice)))
        .collect();
    let mut claimed =
        ctx.proceso()
            .claim_available(Phase::Rips, &items, &ctx.claim_options(&req.cargue_id))?;
    let mut jobs = Vec::new();
    for input in open {
        match claimed.remove(&input.invoice) {
            Some(version) => jobs.push(Job { input, version }),
            None => exec.not_processed(
                Skipped {
                    invoice: input.invoice,
                    code: OTHER_MACHINE.to_owned(),
                    detail: "otra máquina tiene la factura en proceso".to_owned(),
                },
                OTHER_MACHINE,
            ),
        }
    }
    Ok(jobs)
}

fn fold(exec: &mut Execution, finished: Finished) {
    match finished {
        Finished::Written(ready) => {
            let (meds, oss) = ready.service_counts();
            exec.report.push(&ready.invoice, Outcome::done());
            exec.written
                .push((ready.invoice.clone(), ready.file_name(), meds, oss));
        }
        Finished::Recorded(skipped) => {
            exec.report
                .push(&skipped.invoice, plan::skip_outcome(&skipped));
            exec.skipped.push(skipped);
        }
        Finished::NotRecorded(skipped) => exec.not_processed_by_code(skipped),
        Finished::Lost(invoice) => exec.not_processed(
            Skipped {
                invoice,
                code: OTHER_MACHINE.to_owned(),
                detail: "la reclamación de la factura fue tomada por otra máquina".to_owned(),
            },
            OTHER_MACHINE,
        ),
        Finished::Interrupted(invoice) => exec.not_processed(
            Skipped {
                invoice,
                code: INTERRUPTED.to_owned(),
                detail: "interrumpido antes de procesar la factura".to_owned(),
            },
            INTERRUPTED,
        ),
    }
}

/// Processes one claimed invoice; its claim is released unless an outcome was
/// stored (Go recorded nothing for the skips that are not `rips` outcomes).
fn work(
    ctx: &AppContext,
    req: &PhaseRequest,
    opts: &RipsOptions,
    own: &HashMap<String, ProcesoState>,
    job: Job,
) -> Result<Finished> {
    let result = plan_and_store(ctx, req, opts, &job);
    if matches!(
        result,
        Err(_) | Ok(Finished::NotRecorded(_) | Finished::Interrupted(_))
    ) {
        let claim = HashMap::from([(job.input.invoice.clone(), job.version)]);
        if let Err(err) = ctx.proceso().release(Phase::Rips, &claim, own) {
            tracing::warn!(invoice = %job.input.invoice, error = %err, "could not release the claim");
        }
    }
    result
}

fn plan_and_store(
    ctx: &AppContext,
    req: &PhaseRequest,
    opts: &RipsOptions,
    job: &Job,
) -> Result<Finished> {
    let invoice = job.input.invoice.as_str();
    if shutdown::requested() {
        return Ok(Finished::Interrupted(invoice.to_owned()));
    }
    let artifacts = ctx.artifacts();
    let rights = derechos::read(&artifacts, &req.cargue_id, invoice)?;
    match plan::plan_invoice(&job.input, rights.as_ref(), opts)? {
        Planned::Ready(ready) => {
            let file = ready.file_name();
            artifacts
                .publish_bytes(&req.cargue_id, &file, ready.json.as_bytes())
                .with_context(|| format!("rips: write {file}"))?;
            store(ctx, job, &Outcome::done(), Finished::Written(ready))
        }
        Planned::Skipped(skipped) if plan::records_skip(&skipped.code) => {
            let outcome = plan::skip_outcome(&skipped);
            store(ctx, job, &outcome, Finished::Recorded(skipped))
        }
        Planned::Skipped(skipped) => Ok(Finished::NotRecorded(skipped)),
    }
}

/// Stores the outcome while our claim is current; `Lost` when another machine
/// took the claim over.
fn store(ctx: &AppContext, job: &Job, outcome: &Outcome, done: Finished) -> Result<Finished> {
    let invoice = &job.input.invoice;
    if ctx
        .proceso()
        .save_outcome(Phase::Rips, invoice, outcome, job.version)?
    {
        Ok(done)
    } else {
        Ok(Finished::Lost(invoice.clone()))
    }
}
