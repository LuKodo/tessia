//! The `conciliar` command: reads the prepared dataset, decides every invoice
//! ([`super::plan`]), asks Medicar for the formula date of the reconcilable
//! ones, rewrites them through the dataset store and records the outcomes
//! (CONC-01..CONC-14).
//!
//! # Multi-machine rules (replace the Go lock files)
//!
//! * Invoices that need rebuilding are excluded across machines with the lease
//!   `cuv:<cargue_id>:<invoice>` (1 h), the same name `cuv generar` must use for
//!   its per-invoice exclusion (Go `.tessia-cuv`). Go failed the command when
//!   the lock was busy; here the invoice is skipped
//!   (`EN_PROCESO_OTRA_MAQUINA`) so other machines can split the work.
//! * Phases `conciliacion` and `fev_valor` of those invoices are claimed in
//!   `proceso_estado` before the lookups; a claim another machine holds skips
//!   the invoice.
//! * The dataset is written with the optimistic versions read at the start:
//!   a changed invoice fails the run with a retryable message.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;

use super::catalog::{Catalog, normalize_code};
use super::files::{read_invoice_xml, remove_stale, txt_has_cuv_published};
use super::medicar::{FormulaClient, MedicarOptions, Outage};
use super::plan::{
    Candidate, CountKey, Counts, Decision, Detail, Finished, Incomplete, OwnState, Pending,
    Recorded, Rules, cuv_existente, decide, finish_pending,
};
use super::rules::cents;
use crate::context::AppContext;
use crate::domain::dataset::{Dataset, Invoice, Preparation};
use crate::domain::{Outcome, Phase, PhaseState};
use crate::fevxml::invoice_lines;
use crate::nt::NtRow;
use crate::phases::common::{PhaseReport, PhaseRequest};
use crate::stores::dataset::DatasetError;
use crate::stores::leases::LeaseGuard;
use crate::stores::proceso::{ClaimItem, ProcesoState};
use crate::util::decimal::parse_max_amount;
use crate::util::par;

/// How long the per-invoice `cuv:` lease is held (Go: 1 h).
pub const CUV_LEASE_TTL: Duration = Duration::from_secs(3600);
/// Default threads reading invoice XMLs from the bucket.
pub const DEFAULT_WORKERS: usize = 8;
/// Skip reason of an invoice another machine is working on.
pub const OTHER_MACHINE: &str = "EN_PROCESO_OTRA_MAQUINA";

/// Reconcilable invoices (index in the dataset, analysis) waiting for Medicar.
type PendingList = Vec<(usize, Pending)>;

/// A follow-up phase (`rips generar`, `cuv generar`); a seam for tests.
pub type FollowupFn = Arc<dyn Fn(&AppContext, &PhaseRequest) -> Result<PhaseReport> + Send + Sync>;

/// One outage memory per process: the per-invoice runs of a `proceso run`
/// share it, so one dead portal costs one retry sequence, not one per invoice.
static PROCESO_OUTAGE: LazyLock<Arc<Outage>> = LazyLock::new(|| Arc::new(Outage::new()));

/// How a run behaves.
#[derive(Clone)]
pub struct ConciliarOptions {
    /// Run `rips generar` and `cuv generar` for the rebuilt invoices
    /// (standalone command only; `proceso run` routes the successors itself).
    pub followup: bool,
    /// Where an unavailable Medicar portal is remembered.
    pub outage: Arc<Outage>,
    /// Medicar settings; `None` = read `medicar.*` from the configuration.
    pub medicar: Option<MedicarOptions>,
    pub rips: FollowupFn,
    pub cuv: FollowupFn,
}

impl ConciliarOptions {
    /// The standalone command: follow-up on, a private outage memory.
    pub fn standalone() -> Self {
        Self {
            followup: true,
            outage: Arc::new(Outage::new()),
            medicar: None,
            rips: Arc::new(crate::phases::rips::run_phase),
            cuv: Arc::new(crate::phases::cuv::run_phase),
        }
    }

    /// Under `proceso run`: no follow-up, the process-wide outage memory.
    pub fn for_proceso() -> Self {
        Self {
            followup: false,
            outage: PROCESO_OUTAGE.clone(),
            ..Self::standalone()
        }
    }
}

/// Everything a run did.
#[derive(Debug, Clone, Default)]
pub struct Execution {
    /// Outcomes stored for phase `conciliacion` and the invoices skipped.
    pub report: PhaseReport,
    pub counts: Counts,
    /// The invoices behind the non-zero counters.
    pub details: Vec<Detail>,
    /// Console lines, in processing order.
    pub lines: Vec<String>,
    /// Invoices whose lines were rebuilt.
    pub rebuilt: Vec<String>,
}

impl Execution {
    /// The error that makes the command exit non-zero, if any.
    pub fn incomplete(&self) -> Option<Incomplete> {
        self.counts.incomplete(self.details.clone())
    }

    fn line(&mut self, text: String) {
        self.lines.push(text);
    }

    fn skip(&mut self, invoice: &str, reason: &str, line: String) {
        self.counts.add(CountKey::Omitidas);
        self.report.skip(invoice, reason);
        self.line(line);
    }

    fn record(&mut self, invoice: &str, r: Recorded) {
        self.counts.add(r.key);
        self.line(r.line);
        self.details.push(r.detail);
        self.report.push(invoice, r.outcome);
    }
}

/// `proceso run` entry point: no follow-up, no printing (the lines are in
/// [`run_phase_detailed`]'s result). A run with unreconciled invoices is not an
/// error here; use [`run_phase_detailed`] to read [`Execution::incomplete`].
pub fn run_phase(ctx: &AppContext, req: &PhaseRequest) -> Result<PhaseReport> {
    Ok(run_phase_detailed(ctx, req, &ConciliarOptions::for_proceso())?.report)
}

/// [`run_phase`] with explicit options and the full [`Execution`].
pub fn run_phase_detailed(
    ctx: &AppContext,
    req: &PhaseRequest,
    opts: &ConciliarOptions,
) -> Result<Execution> {
    execute(ctx, req, opts)
}

/// The command line: prints the lines and the summary, runs the follow-up and
/// fails when any invoice ended blocked, different, in manual review or failed.
pub fn run(ctx: &AppContext, args: super::cli::Args) -> Result<()> {
    let req = args.common.to_request()?;
    run_request(ctx, &req, &ConciliarOptions::standalone())
}

/// [`run`] for an explicit request and options.
pub fn run_request(ctx: &AppContext, req: &PhaseRequest, opts: &ConciliarOptions) -> Result<()> {
    let exec = execute(ctx, req, opts)?;
    for line in &exec.lines {
        println!("{line}");
    }
    println!("{}", exec.counts.summary_line());
    if opts.followup && !exec.rebuilt.is_empty() {
        followup(ctx, req, opts, &exec.rebuilt)?;
    }
    match exec.incomplete() {
        Some(err) => Err(err.into()),
        None => Ok(()),
    }
}

// ------------------------------------------------------------------ execute

fn execute(ctx: &AppContext, req: &PhaseRequest, opts: &ConciliarOptions) -> Result<Execution> {
    let ds = ctx
        .dataset()
        .read_current(&req.cargue_id, req.invoices.as_deref())?
        .ok_or_else(|| {
            anyhow!(
                "el cargue {} no tiene dataset preparado; ejecute convenio preparar",
                req.cargue_id
            )
        })?;
    let max_cents = cap_cents(ctx, &ds.preparation)?;
    let names: Vec<String> = ds.invoices.iter().map(|i| i.invoice.clone()).collect();
    let prev = ctx.proceso().load(Phase::Conciliacion, Some(&names))?;
    let own = own_states(req, &prev);

    let mut exec = Execution::default();
    let xmls = load_xmls(ctx, req, &ds, &own)?;
    let catalog = load_catalog(ctx, &ds.preparation.nt_version, &xmls)?;
    let rules = Rules {
        catalog: &catalog,
        max_cents,
    };
    let mut pending: PendingList = Vec::new();
    for (i, inv) in ds.invoices.iter().enumerate() {
        let xml = xmls.get(&inv.invoice).and_then(|x| x.as_deref());
        let cand = Candidate {
            invoice: inv,
            xml,
            own: own.get(&inv.invoice),
        };
        match decide(&cand, &rules) {
            Decision::Skip { line } => {
                let reason = skip_reason(&line).to_owned();
                exec.skip(&inv.invoice, &reason, line);
            }
            Decision::Record(r) => exec.record(&inv.invoice, r),
            Decision::Pending(p) => pending.push((i, *p)),
        }
    }
    let pending = drop_existing_cuv(ctx, req, &ds, pending, &mut exec)?;
    store_recorded(ctx, req, &prev, &exec)?;
    if !pending.is_empty() {
        reconcile(ctx, req, opts, &ds, pending, &mut exec)?;
    }
    Ok(exec)
}

/// The machine-readable skip reason of a console line.
fn skip_reason(line: &str) -> &str {
    let rest = line.trim_start().strip_prefix("skip ").unwrap_or(line);
    rest.split_whitespace().nth(1).unwrap_or("OMITIDA")
}

/// Cap in cents: the dataset's, else `convenio.max_invoice_amount`; unchecked
/// (with a warning) when neither is set.
fn cap_cents(ctx: &AppContext, prep: &Preparation) -> Result<Option<i64>> {
    let amount = match prep.max_amount {
        Some(a) => Some(a),
        None => match ctx.settings.get("convenio.max_invoice_amount") {
            Some(text) => Some(parse_max_amount(text).context("monto maximo")?),
            None => None,
        },
    };
    if amount.is_none() {
        tracing::warn!(
            "no per-invoice cap (the dataset's monto_maximo_factura / convenio.max_invoice_amount): BLOQUEADA_MONTO is not checked"
        );
    }
    Ok(amount.map(cents))
}

/// Own conciliación states, only without `--invoices` (which retries them).
fn own_states(
    req: &PhaseRequest,
    prev: &HashMap<String, ProcesoState>,
) -> HashMap<String, OwnState> {
    if req.invoices.is_some() {
        return HashMap::new();
    }
    prev.iter()
        .map(|(k, s)| {
            let own = OwnState {
                state: s.state,
                motivo: s.motivo.clone(),
            };
            (k.clone(), own)
        })
        .collect()
}

/// Reads the XML of every invoice that is not skipped by its own state.
fn load_xmls(
    ctx: &AppContext,
    req: &PhaseRequest,
    ds: &Dataset,
    own: &HashMap<String, OwnState>,
) -> Result<HashMap<String, Option<Vec<u8>>>> {
    let artifacts = ctx.artifacts();
    let skipped = |name: &str| {
        own.get(name)
            .is_some_and(|o| matches!(o.state, PhaseState::Excluded | PhaseState::RevisionManual))
    };
    let todo: Vec<String> = ds
        .invoices
        .iter()
        .map(|i| i.invoice.clone())
        .filter(|n| !skipped(n))
        .collect();
    let read = par::map(todo.clone(), req.workers_or(DEFAULT_WORKERS), |name| {
        read_invoice_xml(&artifacts, &req.cargue_id, &name)
    });
    todo.into_iter()
        .zip(read)
        .map(|(name, xml)| Ok((name, xml?)))
        .collect()
}

/// Builds the catalog from the NT version pinned at preparation time. The NT
/// has no "all rows" call, so it asks for the rows of every CUM that can end
/// an XML line identifier (`<PLU>-<CUM>`, the CUM may contain dashes).
fn load_catalog(
    ctx: &AppContext,
    version: &str,
    xmls: &HashMap<String, Option<Vec<u8>>>,
) -> Result<Catalog> {
    if version.is_empty() {
        bail!("el dataset no tiene versión de nota técnica fijada; ejecute convenio preparar");
    }
    if !ctx.nt.versions()?.iter().any(|v| v == version) {
        bail!("la versión {version} de la nota técnica del cargue no está en Redis");
    }
    let mut seen = HashSet::new();
    let mut rows: Vec<NtRow> = Vec::new();
    for xml in xmls.values().flatten() {
        let Ok(lines) = invoice_lines(xml) else {
            continue;
        };
        for line in lines {
            for (i, _) in line.identifier.trim().match_indices('-') {
                let cum = &line.identifier.trim()[i + 1..];
                if cum.is_empty() || !seen.insert(cum.to_owned()) {
                    continue;
                }
                rows.extend(lookup_cum(ctx, version, cum)?);
            }
        }
    }
    Ok(Catalog::from_nt_rows(&rows))
}

fn lookup_cum(ctx: &AppContext, version: &str, cum: &str) -> Result<Vec<NtRow>> {
    let mut rows = ctx.nt.lookup(version, "cum_facturar", cum)?;
    let normalized = normalize_code(cum);
    if rows.is_empty() && normalized != cum {
        rows = ctx.nt.lookup(version, "cum_facturar", &normalized)?;
    }
    Ok(rows)
}

/// Invoices whose `<F>.txt` already holds a CUV go to manual review without
/// querying Medicar (CONC-08).
fn drop_existing_cuv(
    ctx: &AppContext,
    req: &PhaseRequest,
    ds: &Dataset,
    pending: PendingList,
    exec: &mut Execution,
) -> Result<PendingList> {
    let artifacts = ctx.artifacts();
    let mut active = Vec::with_capacity(pending.len());
    for (i, p) in pending {
        let name = &ds.invoices[i].invoice;
        if txt_has_cuv_published(&artifacts, &req.cargue_id, name)? {
            exec.record(name, cuv_existente(name));
        } else {
            active.push((i, p));
        }
    }
    Ok(active)
}

/// Stores the outcomes decided without Medicar (Go `RecordPlanned`).
fn store_recorded(
    ctx: &AppContext,
    req: &PhaseRequest,
    prev: &HashMap<String, ProcesoState>,
    exec: &Execution,
) -> Result<()> {
    let outcomes: Vec<(String, Outcome)> = exec
        .report
        .outcomes
        .iter()
        .map(|o| (o.invoice.clone(), o.outcome.clone()))
        .collect();
    if outcomes.is_empty() {
        return Ok(());
    }
    ctx.proceso()
        .record(Phase::Conciliacion, prev, &outcomes, Some(&req.cargue_id))?;
    Ok(())
}

// ---------------------------------------------------------------- reconcile

/// Claims held on one invoice: the `proceso_estado` versions of both phases.
struct Claims {
    conc: HashMap<String, i64>,
    fev: HashMap<String, i64>,
    prev_conc: HashMap<String, ProcesoState>,
    prev_fev: HashMap<String, ProcesoState>,
}

impl Claims {
    fn release(&self, ctx: &AppContext, names: &[String]) {
        let pick = |m: &HashMap<String, i64>| -> HashMap<String, i64> {
            names
                .iter()
                .filter_map(|n| m.get(n).map(|v| (n.clone(), *v)))
                .collect()
        };
        let proceso = ctx.proceso();
        for (phase, held, prev) in [
            (Phase::Conciliacion, pick(&self.conc), &self.prev_conc),
            (Phase::FevValor, pick(&self.fev), &self.prev_fev),
        ] {
            if let Err(err) = proceso.release(phase, &held, prev) {
                tracing::warn!(error = %err, "could not release the claims of conciliar");
            }
        }
    }
}

fn reconcile(
    ctx: &AppContext,
    req: &PhaseRequest,
    opts: &ConciliarOptions,
    ds: &Dataset,
    pending: PendingList,
    exec: &mut Execution,
) -> Result<()> {
    let medicar = match &opts.medicar {
        Some(m) => m.clone(),
        None => MedicarOptions::from_settings(&ctx.settings)?,
    };
    // The leases live until the dataset is written and the stale files removed.
    let (_leases, pending) = lease_pending(ctx, req, ds, pending, exec)?;
    let (claims, pending) = claim_pending(ctx, req, ds, pending, exec)?;
    if pending.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = pending
        .iter()
        .map(|(i, _)| ds.invoices[*i].invoice.clone())
        .collect();
    let result = lookup_and_write(ctx, req, opts, medicar, ds, pending, &claims, exec);
    if result.is_err() {
        claims.release(ctx, &names);
    }
    result
}

fn lease_pending(
    ctx: &AppContext,
    req: &PhaseRequest,
    ds: &Dataset,
    pending: PendingList,
    exec: &mut Execution,
) -> Result<(Vec<LeaseGuard>, PendingList)> {
    let leases = ctx.leases();
    let mut guards = Vec::new();
    let mut active = Vec::new();
    for (i, p) in pending {
        let name = &ds.invoices[i].invoice;
        match leases.guard(&lease_name(&req.cargue_id, name), &ctx.owner, CUV_LEASE_TTL)? {
            Some(g) => {
                guards.push(g);
                active.push((i, p));
            }
            None => exec.skip(
                name,
                OTHER_MACHINE,
                format!("  skip {name} {OTHER_MACHINE} (otra máquina procesa la factura)"),
            ),
        }
    }
    Ok((guards, active))
}

/// Name of the per-invoice lease shared with `cuv generar`.
pub fn lease_name(cargue_id: &str, invoice: &str) -> String {
    format!("cuv:{cargue_id}:{invoice}")
}

fn claim_pending(
    ctx: &AppContext,
    req: &PhaseRequest,
    ds: &Dataset,
    pending: PendingList,
    exec: &mut Execution,
) -> Result<(Claims, PendingList)> {
    let proceso = ctx.proceso();
    let names: Vec<String> = pending
        .iter()
        .map(|(i, _)| ds.invoices[*i].invoice.clone())
        .collect();
    let prev_fev = proceso.load(Phase::FevValor, Some(&names))?;
    let prev_conc = proceso.load(Phase::Conciliacion, Some(&names))?;
    let opts = ctx.claim_options(&req.cargue_id);
    let items = |prev: &HashMap<String, ProcesoState>, only: &[String]| -> Vec<ClaimItem> {
        only.iter()
            .map(|n| ClaimItem::from_loaded(n, prev.get(n)))
            .collect()
    };
    let fev = proceso.claim_available(Phase::FevValor, &items(&prev_fev, &names), &opts)?;
    let wanted: Vec<String> = names
        .iter()
        .filter(|n| fev.contains_key(*n))
        .cloned()
        .collect();
    let conc = proceso.claim_available(Phase::Conciliacion, &items(&prev_conc, &wanted), &opts)?;
    let claims = Claims {
        conc,
        fev,
        prev_conc,
        prev_fev,
    };
    // Claimed in fev_valor but not in conciliacion: give the first back.
    let half: Vec<String> = wanted
        .iter()
        .filter(|n| !claims.conc.contains_key(*n))
        .cloned()
        .collect();
    claims.release(ctx, &half);
    let mut active = Vec::new();
    for (i, p) in pending {
        let name = &ds.invoices[i].invoice;
        if claims.conc.contains_key(name) && claims.fev.contains_key(name) {
            active.push((i, p));
        } else {
            exec.skip(
                name,
                OTHER_MACHINE,
                format!("  skip {name} {OTHER_MACHINE} (otra máquina procesa la factura)"),
            );
        }
    }
    Ok((claims, active))
}

fn first_line_query(inv: &Invoice) -> Option<(&str, &str, &str, &str)> {
    let first = inv.lines.first()?;
    let reference = if first.invoice_raw.is_empty() {
        inv.invoice.as_str()
    } else {
        first.invoice_raw.as_str()
    };
    Some((&first.doc_number, &first.ssc, reference, &first.article))
}

#[allow(clippy::too_many_arguments)]
fn lookup_and_write(
    ctx: &AppContext,
    req: &PhaseRequest,
    opts: &ConciliarOptions,
    mut medicar: MedicarOptions,
    ds: &Dataset,
    pending: PendingList,
    claims: &Claims,
    exec: &mut Execution,
) -> Result<()> {
    medicar.outage = Some(opts.outage.clone());
    let client = FormulaClient::new(medicar).map_err(|e| anyhow!("{e}"))?;
    let mut finished: Vec<(usize, Finished)> = Vec::new();
    let mut unprocessed: Vec<String> = Vec::new();
    for (i, p) in pending {
        let inv = &ds.invoices[i];
        if crate::shutdown::requested() {
            unprocessed.push(inv.invoice.clone());
            continue;
        }
        let answer = match first_line_query(inv) {
            Some((doc, ssc, reference, article)) => client.lookup(doc, ssc, reference, article),
            None => Err(super::medicar::MedicarError::Permanent(
                "medicar: la factura no tiene lineas para consultar".into(),
            )),
        };
        finished.push((i, finish_pending(inv, &p, answer, Utc::now())));
    }
    write_rebuilt(ctx, req, ds, &finished)?;
    store_finished(ctx, ds, &finished, claims, exec)?;
    claims.release(ctx, &unprocessed);
    Ok(())
}

/// Writes the rebuilt invoices, each against the version read, then removes
/// the files that still hold the old values.
fn write_rebuilt(
    ctx: &AppContext,
    req: &PhaseRequest,
    ds: &Dataset,
    finished: &[(usize, Finished)],
) -> Result<()> {
    let updates: Vec<(Invoice, i64)> = finished
        .iter()
        .filter_map(|(i, f)| f.rebuilt.clone().map(|r| (r, ds.invoices[*i].version)))
        .collect();
    if updates.is_empty() {
        return Ok(());
    }
    ctx.dataset()
        .update_invoices(ds.cargue, &updates)
        .map_err(|e| match e {
            DatasetError::VersionConflict { .. } => {
                anyhow!("an invoice changed while conciliar worked on it; rerun the command: {e}")
            }
            other => anyhow!(other),
        })?;
    let artifacts = ctx.artifacts();
    for (inv, _) in &updates {
        remove_stale(&artifacts, &req.cargue_id, &inv.invoice)?;
    }
    Ok(())
}

fn store_finished(
    ctx: &AppContext,
    ds: &Dataset,
    finished: &[(usize, Finished)],
    claims: &Claims,
    exec: &mut Execution,
) -> Result<()> {
    let proceso = ctx.proceso();
    for (i, f) in finished {
        let name = &ds.invoices[*i].invoice;
        for (phase, outcome, held) in [
            (Phase::Conciliacion, &f.conciliacion, &claims.conc),
            (Phase::FevValor, &f.fev_valor, &claims.fev),
        ] {
            let version = held.get(name).copied().unwrap_or_default();
            if !proceso.save_outcome(phase, name, outcome, version)? {
                tracing::warn!(invoice = %name, phase = %phase, "the claim was taken over; result dropped");
            }
        }
        exec.counts.add(f.key);
        exec.line(f.line.clone());
        exec.details.extend(f.detail.clone());
        exec.report.push(name, f.conciliacion.clone());
        if f.rebuilt.is_some() {
            exec.rebuilt.push(name.clone());
        }
    }
    Ok(())
}

// ----------------------------------------------------------------- follow-up

/// Standalone follow-up (CONC-13): `rips generar` then `cuv generar` for the
/// rebuilt invoices, each required to end `DONE`, and a CUV in every `<F>.txt`.
fn followup(
    ctx: &AppContext,
    req: &PhaseRequest,
    opts: &ConciliarOptions,
    rebuilt: &[String],
) -> Result<()> {
    let sub = PhaseRequest {
        cargue_id: req.cargue_id.clone(),
        invoices: Some(rebuilt.to_vec()),
        workers: None,
    };
    (opts.rips)(ctx, &sub).map_err(|e| anyhow!("conciliar follow-up rips generar: {e}"))?;
    require_done(ctx, Phase::Rips, rebuilt)
        .map_err(|e| anyhow!("conciliar follow-up RIPS: {e}"))?;
    (opts.cuv)(ctx, &sub).map_err(|e| anyhow!("conciliar follow-up cuv generar: {e}"))?;
    require_done(ctx, Phase::Cuv, rebuilt).map_err(|e| anyhow!("conciliar follow-up CUV: {e}"))?;
    let artifacts = ctx.artifacts();
    for inv in rebuilt {
        let has = txt_has_cuv_published(&artifacts, &req.cargue_id, inv)
            .map_err(|e| anyhow!("conciliar follow-up CUV: read {inv}.txt: {e}"))?;
        if !has {
            bail!("conciliar follow-up CUV: {inv} did not produce a CUV in {inv}.txt");
        }
    }
    Ok(())
}

fn require_done(ctx: &AppContext, phase: Phase, invoices: &[String]) -> Result<()> {
    let states = ctx.proceso().load(phase, Some(invoices))?;
    for inv in invoices {
        let (status, reason) = states
            .get(inv)
            .map_or(("", ""), |s| (s.state.as_str(), s.motivo.as_str()));
        if status != PhaseState::Done.as_str() {
            bail!("{inv} did not reach DONE (status={status:?}): {reason}");
        }
    }
    Ok(())
}
