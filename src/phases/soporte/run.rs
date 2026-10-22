//! The phase: claims, gate, worker pool, per-invoice build and state recording.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use super::assets;
use super::cpscan::{self, Missing};
use super::decide::{CODE_EXISTS, CODE_INVALID_INVOICE, CODE_OK, state_of};
use super::generator::{self, Request};
use super::input::{Header, SoporteInput};
use super::zipfile;
use crate::context::AppContext;
use crate::domain::{Outcome, Phase, PhaseState, normalize_invoice, valid_invoice};
use crate::phases::common::{PhaseReport, PhaseRequest};
use crate::shutdown;
use crate::stores::proceso::{ClaimItem, ProcesoState};
use crate::util::par;

/// Default and maximum invoices in flight (`--workers`).
pub const DEFAULT_WORKERS: usize = 20;
pub const MAX_WORKERS: usize = 35;

/// Reason of an invoice another machine is working on.
pub const OTHER_MACHINE: &str = "EN_PROCESO_OTRA_MAQUINA";

/// How long the per-invoice lease lasts (a build takes seconds).
const LEASE_TTL: Duration = Duration::from_secs(15 * 60);

/// Options of one run.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Rebuild even when the ZIP exists, replacing it (only on an explicit call).
    pub regenerate: bool,
}

/// What a run did, with the console lines.
#[derive(Debug, Clone, Default)]
pub struct SoporteRun {
    pub report: PhaseReport,
    /// One line per invoice, in processing order (already indented).
    pub lines: Vec<String>,
    pub ok: usize,
    pub existing: usize,
    pub failed: usize,
    /// Infrastructure failures (bucket, database): their claims were released.
    pub errors: Vec<String>,
}

impl SoporteRun {
    /// `soporte: ok=N existing=N failed=N`.
    pub fn summary_line(&self) -> String {
        format!(
            "soporte: ok={} existing={} failed={}",
            self.ok, self.existing, self.failed
        )
    }
}

/// The NIT of the file name: `zonaser.organization_nit`, else `company.nit_emisor`.
pub fn nit(ctx: &AppContext) -> String {
    ctx.settings
        .get("zonaser.organization_nit")
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map_or_else(
            || {
                ctx.settings
                    .get_or("company.nit_emisor", "806011811")
                    .to_owned()
            },
            str::to_owned,
        )
}

/// `soporte.signatures_enabled=true` (case-insensitive; default off).
pub fn signatures_enabled(ctx: &AppContext) -> bool {
    ctx.settings
        .get("soporte.signatures_enabled")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
}

/// Runs the phase over `inputs` (every invoice of the cargue's dataset).
pub fn run_inputs(
    ctx: &AppContext,
    req: &PhaseRequest,
    options: Options,
    inputs: Vec<SoporteInput>,
) -> Result<SoporteRun> {
    let workers = match req.workers {
        Some(n) if !(1..=MAX_WORKERS).contains(&n) => {
            bail!("--workers must be between 1 and {MAX_WORKERS}")
        }
        Some(n) => n,
        None => DEFAULT_WORKERS,
    };
    let selected = select(req, inputs)?;
    let names: Vec<String> = selected.iter().map(|i| i.invoice.clone()).collect();
    let loaded = ctx.proceso().load(Phase::Soporte, Some(&names))?;
    let mut run = SoporteRun::default();
    let mut items = Vec::new();
    for input in &selected {
        let state = loaded.get(&input.invoice);
        if req.invoices.is_none() && state.is_some_and(|s| s.state == PhaseState::RevisionManual) {
            run.lines.push(gate_line(&input.invoice, state));
            run.report.skip(&input.invoice, "REVISION_MANUAL");
            continue;
        }
        items.push(ClaimItem::from_loaded(&input.invoice, state));
    }
    let claimed = ctx.proceso().claim_available(
        Phase::Soporte,
        &items,
        &ctx.claim_options(&req.cargue_id),
    )?;
    let mut work = Vec::new();
    for input in selected {
        if items.iter().all(|i| i.invoice != input.invoice) {
            continue;
        }
        match claimed.get(&input.invoice) {
            Some(version) => work.push((input, *version)),
            None => {
                run.lines
                    .push(format!("  {} {OTHER_MACHINE}", input.invoice));
                run.report.skip(&input.invoice, OTHER_MACHINE);
            }
        }
    }
    let shared = Shared {
        ctx,
        cargue: &req.cargue_id,
        nit: nit(ctx),
        options,
        signatures: signatures_enabled(ctx),
        logo: assets::load_logo(ctx),
    };
    let results = par::map(work, workers, |(input, version)| {
        let work = process(&shared, &input);
        (input.invoice, version, work)
    });
    record(ctx, req, &loaded, results, &mut run);
    Ok(run)
}

/// The invoices to process, in order, validated against `--invoices`.
fn select(req: &PhaseRequest, inputs: Vec<SoporteInput>) -> Result<Vec<SoporteInput>> {
    let mut by_invoice: HashMap<String, SoporteInput> = HashMap::new();
    let mut order = Vec::new();
    for input in inputs {
        if !by_invoice.contains_key(&input.invoice) {
            order.push(input.invoice.clone());
            by_invoice.insert(input.invoice.clone(), input);
        }
    }
    let wanted: Vec<String> = req.invoices.as_ref().map_or(order, |names| {
        names.iter().map(|n| normalize_invoice(n)).collect()
    });
    wanted
        .into_iter()
        .map(|invoice| {
            by_invoice
                .remove(&invoice)
                .ok_or_else(|| anyhow!("invoice {invoice} is not in the dataset of the load"))
        })
        .collect()
}

/// `<inv> REVISION_MANUAL: soporte REVISION_MANUAL: <reason> (reprocess with --invoices)`.
fn gate_line(invoice: &str, state: Option<&ProcesoState>) -> String {
    let mut detail = "soporte REVISION_MANUAL".to_owned();
    if let Some(state) = state.filter(|s| !s.motivo.is_empty()) {
        detail.push_str(": ");
        detail.push_str(&state.motivo);
    }
    format!("  {invoice} REVISION_MANUAL: {detail} (reprocess with --invoices)")
}

struct Shared<'a> {
    ctx: &'a AppContext,
    cargue: &'a str,
    nit: String,
    options: Options,
    signatures: bool,
    logo: std::result::Result<Vec<u8>, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ok,
    Existing,
    Other,
}

enum Work {
    /// A result to store.
    Done {
        line: String,
        outcome: Outcome,
        kind: Kind,
    },
    /// Not started (interrupted, or another machine holds the invoice): give the claim back.
    Release(String),
    /// Infrastructure failure: give the claim back and report.
    Failed(String),
}

fn process(shared: &Shared<'_>, input: &SoporteInput) -> Work {
    if shutdown::requested() {
        return Work::Release("INTERRUMPIDO".into());
    }
    match process_invoice(shared, input) {
        Ok(work) => work,
        Err(err) => Work::Failed(format!("{err:#}")),
    }
}

fn process_invoice(shared: &Shared<'_>, input: &SoporteInput) -> Result<Work> {
    let (ctx, invoice) = (shared.ctx, input.invoice.as_str());
    if !valid_invoice(invoice) {
        return Ok(rejected(
            invoice,
            CODE_INVALID_INVOICE,
            "invalid invoice number",
        ));
    }
    let lease = format!("soporte:{}:{invoice}", shared.cargue);
    let Some(_guard) = ctx.leases().guard(&lease, &ctx.owner, LEASE_TTL)? else {
        return Ok(Work::Release(OTHER_MACHINE.into()));
    };
    let zip = zipfile::zip_rel(&shared.nit, invoice);
    let exists = non_empty(ctx, shared.cargue, &zip)?;
    if exists && !shared.options.regenerate {
        return Ok(Work::Done {
            line: format!("  {invoice} {CODE_EXISTS}"),
            outcome: Outcome::done(),
            kind: Kind::Existing,
        });
    }
    let header = Header::from_input(input);
    let loaded = cpscan::load(ctx, shared.cargue, invoice)?;
    let (stored, payload, problem) = match &loaded {
        Ok(l) => (Some(&l.stored), l.payload.as_slice(), None),
        Err(Missing::NotFetched) => (None, &[][..], None),
        Err(Missing::Problem(p)) => (None, &[][..], Some(p.as_str())),
    };
    let invoice_pdf = invoice_pdf(ctx, shared.cargue, invoice)?;
    let signature = if shared.signatures {
        assets::existing_signature(ctx, &header)?
    } else {
        None
    };
    let request = Request {
        header: &header,
        stored,
        payload,
        problem,
        invoice_pdf: &invoice_pdf,
        signature: signature.as_deref(),
        logo: shared.logo.as_deref().map_err(String::as_str),
    };
    let built = generator::build(&request, &|h: &Header| assets::manual_signature(ctx, h));
    if built.code != CODE_OK {
        return Ok(failure(invoice, built.code, &built.detail, exists));
    }
    let zip_bytes = zipfile::build_zip(
        &zipfile::pdf_name(&shared.nit, invoice),
        &built.pdf,
        zipfile::now(),
    )?;
    ctx.artifacts()
        .publish_bytes(shared.cargue, &zip, &zip_bytes)
        .with_context(|| format!("no se pudo publicar {zip}"))?;
    Ok(success(invoice, &built, exists, header.items.len()))
}

/// `<F>.pdf` of the cargue, or empty when it is missing or empty.
fn invoice_pdf(ctx: &AppContext, cargue: &str, invoice: &str) -> Result<Vec<u8>> {
    let rel = format!("{invoice}.pdf");
    if !non_empty(ctx, cargue, &rel)? {
        return Ok(Vec::new());
    }
    ctx.artifacts().fetch_bytes(cargue, &rel)
}

/// The artifact is indexed and not empty.
fn non_empty(ctx: &AppContext, cargue: &str, rel: &str) -> Result<bool> {
    Ok(ctx
        .artefactos()
        .get(cargue, rel)?
        .is_some_and(|a| a.size > 0))
}

/// A result that never reaches the builder.
fn rejected(invoice: &str, code: &'static str, detail: &str) -> Work {
    failure(invoice, code, detail, false)
}

/// `<inv> <CODE> (detail)` and the state of the code. A rebuild that failed
/// keeps the previous ZIP and says so.
fn failure(invoice: &str, code: &'static str, detail: &str, replaced: bool) -> Work {
    let mut detail = detail.to_owned();
    if replaced {
        if !detail.is_empty() {
            detail.push_str("; ");
        }
        detail.push_str("ZIP anterior conservado");
    }
    let mut line = format!("  {invoice} {code}");
    let mut motivo = code.to_owned();
    if !detail.is_empty() {
        line.push_str(&format!(" ({detail})"));
        motivo.push_str(&format!(": {detail}"));
    }
    Work::Done {
        line,
        outcome: Outcome::new(state_of(code), motivo),
        kind: Kind::Other,
    }
}

/// The ZIP was written: `<inv> OK rama=X [regenerado (ZIP reemplazado)] [(detail)] [note]`;
/// the note is kept as the reason of the DONE state.
fn success(invoice: &str, built: &generator::Built, replaced: bool, deliveries: usize) -> Work {
    let branch = built.branch.map_or("", |b| b.as_str());
    let mut line = format!("  {invoice} {CODE_OK} rama={branch}");
    if replaced {
        line.push_str(" regenerado (ZIP reemplazado)");
        tracing::info!(invoice, deliveries, "soporte regenerated");
    }
    if !built.detail.is_empty() {
        line.push_str(&format!(" ({})", built.detail));
    }
    if !built.note.is_empty() {
        line.push_str(&format!(" [{}]", built.note));
    }
    Work::Done {
        line,
        outcome: Outcome::new(PhaseState::Done, built.note.clone()),
        kind: Kind::Ok,
    }
}

/// Stores the outcomes (dropping the ones whose claim was taken over) and gives
/// back the claims of what was not done.
fn record(
    ctx: &AppContext,
    req: &PhaseRequest,
    loaded: &HashMap<String, ProcesoState>,
    results: Vec<(String, i64, Work)>,
    run: &mut SoporteRun,
) {
    let store = ctx.proceso();
    let mut release: HashMap<String, i64> = HashMap::new();
    for (invoice, version, work) in results {
        match work {
            Work::Done {
                line,
                outcome,
                kind,
            } => match store.save_outcome(Phase::Soporte, &invoice, &outcome, version) {
                Ok(true) => {
                    run.lines.push(line);
                    count(run, kind);
                    run.report.push(&invoice, outcome);
                }
                Ok(false) => {
                    run.lines.push(format!("  {invoice} {OTHER_MACHINE}"));
                    run.report.skip(&invoice, OTHER_MACHINE);
                }
                Err(err) => {
                    release.insert(invoice.clone(), version);
                    run.errors.push(format!("{invoice}: {err}"));
                    run.report.skip(&invoice, &format!("ERROR: {err}"));
                }
            },
            Work::Release(reason) => {
                release.insert(invoice.clone(), version);
                run.lines.push(format!("  {invoice} {reason}"));
                run.report.skip(&invoice, &reason);
            }
            Work::Failed(message) => {
                release.insert(invoice.clone(), version);
                run.lines.push(format!("  {invoice} ERROR: {message}"));
                run.errors.push(format!("{invoice}: {message}"));
                run.report.skip(&invoice, &format!("ERROR: {message}"));
            }
        }
    }
    if release.is_empty() {
        return;
    }
    if let Err(err) = store.release(Phase::Soporte, &release, loaded) {
        tracing::warn!(cargue = req.cargue_id, error = %err, "could not release soporte claims");
    }
}

fn count(run: &mut SoporteRun, kind: Kind) {
    match kind {
        Kind::Ok => run.ok += 1,
        Kind::Existing => run.existing += 1,
        Kind::Other => run.failed += 1,
    }
}
