//! `cuv generar`: sends the FEV XML and the RIPS JSON of each invoice to
//! SISPRO (`CargarFevRips`) and stores the answer as `<F>.txt` /
//! `<F>_ERROR.txt` / `<F>_RESPUESTA_NO_CORRESPONDE.txt` (Go `cmd_cuv.go`).
//!
//! Per invoice, in this order: gate (only without `--invoices`), claim in
//! `proceso_estado` (machines split the work), the lease
//! [`super::lease_name`] (excludes `fev ajustar` and other machines on the same
//! invoice), plan (skip codes, XML source, `prepare_ministerio`, payload), write
//! the adjusted XML and ZIP back, send, interpret, sweep, publish, store the
//! outcome. Files live in the bucket (`Artifacts`); nothing touches a fixed
//! local folder.

use std::collections::HashMap;
use std::sync::{LazyLock, OnceLock};

use anyhow::{Result, anyhow, bail};
use regex::Regex;

use super::availability::{TokenFailureError, TokenPolicy, TokenTally, check_token_failures};
use super::model::{build_payload, interpret, regimen, rejection_summary};
use super::sispro::{SisproClient, SisproOptions};
use super::sweep::sweep;
use super::txt::txt_has_cuv;
use super::{LEASE_TTL, SKIP_OTHER_MACHINE, lease_name};
use crate::context::AppContext;
use crate::domain::phase::gate;
use crate::domain::{Outcome, Phase};
use crate::fevxml::{self, FevxmlError, PERIODO_INICIO_MINISTERIO, Params};
use crate::phases::common::{PhaseReport, PhaseRequest};
use crate::storage::Artifacts;
use crate::stores::proceso::{ClaimItem, ProcesoState};
use crate::util::par;

/// Invoices in flight when `--workers` is not given.
pub const DEFAULT_WORKERS: usize = 40;
/// Most invoices in flight.
pub const MAX_WORKERS: usize = 40;

/// `^[A-Za-z]{2,4}[0-9]+\.json$`: the RIPS artifacts that name an invoice.
static RIPS_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z]{2,4}[0-9]+\.json$").unwrap_or_else(|_| unreachable!("constant regex"))
});

/// An explicitly requested invoice has no RIPS JSON (Go `MissingRIPSJSONError`).
/// Callers can tell this invoice-local condition from I/O and configuration
/// failures with `err.downcast_ref::<MissingRipsJsonError>()`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("RIPS_JSON_FALTA: missing RIPS JSON for invoice {invoice}")]
pub struct MissingRipsJsonError {
    pub invoice: String,
}

/// Everything a `cuv generar` run did.
#[derive(Debug, Default)]
pub struct Execution {
    pub report: PhaseReport,
    /// Console lines, in invoice order.
    pub lines: Vec<String>,
    /// Invoices that reached `CargarFevRips` (or failed to log in first).
    pub submitted: usize,
    /// Of those, the ones whose `<F>.txt` now holds a CUV.
    pub with_cuv: usize,
    /// Infrastructure failures (bucket, database, missing config): the claim of
    /// each was given back, the others were stored.
    pub errors: Vec<anyhow::Error>,
}

/// One invoice to process: the file stem (`<F>`) and the normalized key of
/// `proceso_estado`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    stem: String,
    key: String,
}

/// The invoices that have a root-level `<F>.json` artifact, sorted.
fn rips_invoices(index: &HashMap<String, i64>) -> Vec<Target> {
    let mut stems: Vec<&str> = index
        .keys()
        .filter(|rel| RIPS_FILE.is_match(rel))
        .filter_map(|rel| rel.strip_suffix(".json"))
        .collect();
    stems.sort_unstable();
    stems
        .into_iter()
        .map(|stem| Target {
            stem: stem.to_owned(),
            key: crate::domain::normalize_invoice(stem),
        })
        .collect()
}

/// Applies `--invoices`: the requested ones that exist (in the requested
/// order), warning about the unknown ones.
fn select_targets(found: Vec<Target>, requested: Option<&[String]>) -> Result<Vec<Target>> {
    let Some(requested) = requested else {
        return Ok(found);
    };
    let mut selected = Vec::new();
    let mut unknown = Vec::new();
    for invoice in requested {
        match found.iter().find(|t| &t.key == invoice) {
            Some(target) => selected.push(target.clone()),
            None => unknown.push(invoice.clone()),
        }
    }
    if !unknown.is_empty() {
        tracing::warn!(invoices = %unknown.join(","), "requested invoices without <F>.json");
    }
    if selected.is_empty() {
        if let ([only], [_]) = (unknown.as_slice(), requested) {
            return Err(MissingRipsJsonError {
                invoice: only.clone(),
            }
            .into());
        }
        bail!("none of the --invoices has a <F>.json in the folder");
    }
    Ok(selected)
}

/// The first non-empty `<F>_XML_PDF.zip`: `XML/` first, then the older `ZIP/`.
fn existing_zip(index: &HashMap<String, i64>, stem: &str) -> Option<String> {
    ["XML", "ZIP"]
        .iter()
        .map(|dir| format!("{dir}/{stem}_XML_PDF.zip"))
        .find(|rel| index.get(rel).is_some_and(|size| *size > 0))
}

/// What `planCUV` decided for an invoice.
enum Plan {
    Skip { code: &'static str, detail: String },
    Send(Box<SendItem>),
}

struct SendItem {
    payload: Vec<u8>,
    /// The XML to write back (adjusted, or restored from the ZIP).
    xml: Vec<u8>,
    write_xml: bool,
    /// ZIP whose XML entry must be replaced.
    refresh_zip: Option<String>,
    detail: String,
}

/// What a worker stores for an invoice.
struct Processed {
    outcome: Outcome,
    lines: Vec<String>,
    submitted: bool,
    with_cuv: bool,
}

enum Finished {
    Stored(Processed),
    /// Another machine took the invoice (lease held, or our claim went stale).
    OtherMachine(String),
    /// Interrupted before storing anything; the claim was given back.
    Interrupted,
    Failed(anyhow::Error),
}

/// Shared state of one run.
struct Run<'a> {
    ctx: &'a AppContext,
    cargue: &'a str,
    artifacts: Artifacts,
    index: HashMap<String, i64>,
    params: Params,
    previous: HashMap<String, ProcesoState>,
    tally: &'a TokenTally,
    client: OnceLock<Result<SisproClient, String>>,
}

impl Run<'_> {
    /// The SISPRO client, built on first use: a run in which every invoice is
    /// skipped never needs the `minsalud.*` credentials.
    fn client(&self) -> Result<&SisproClient> {
        let built = self.client.get_or_init(|| {
            SisproOptions::from_settings(&self.ctx.settings)
                .and_then(|options| SisproClient::new(options, self.ctx.limiters.get("sispro")))
                .map_err(|e| format!("{e:#}"))
        });
        built.as_ref().map_err(|message| anyhow!("{message}"))
    }

    fn fetch(&self, rel: &str) -> Result<Vec<u8>> {
        self.artifacts.fetch_bytes(self.cargue, rel)
    }

    fn publish(&self, rel: &str, data: &[u8]) -> Result<()> {
        self.artifacts
            .publish_bytes(self.cargue, rel, data)
            .map(|_| ())
    }
}

fn skip_outcome(code: &str, detail: &str) -> Outcome {
    match code {
        "YA_TIENE_CUV" => Outcome::done(),
        "XML_FALTA" => Outcome::failed(format!("{code}{detail}")),
        _ => Outcome::manual_review(format!("{code}{detail}")),
    }
}

/// `planCUV`: the skip codes, the XML source and its preparation.
fn plan(run: &Run<'_>, target: &Target) -> Result<Plan> {
    let stem = &target.stem;
    let skip = |code: &'static str, detail: String| Ok(Plan::Skip { code, detail });
    let txt = format!("{stem}.txt");
    if run.index.contains_key(&txt) && txt_has_cuv(&run.fetch(&txt)?) {
        return skip("YA_TIENE_CUV", String::new());
    }
    let rips = run.fetch(&format!("{stem}.json"))?;
    let xml_rel = format!("{stem}.xml");
    let zip_rel = existing_zip(&run.index, stem);
    let mut write_xml = false;
    let xml = if run.index.get(&xml_rel).is_some_and(|size| *size > 0) {
        run.fetch(&xml_rel)?
    } else if let Some(zip) = &zip_rel {
        let restored = fevxml::read_entry(&run.fetch(zip)?, &xml_rel)
            .ok()
            .flatten();
        let Some(restored) = restored else {
            let name = zip.rsplit('/').next().unwrap_or(zip);
            return skip("XML_FALTA", format!(" (no esta en {name})"));
        };
        write_xml = true;
        restored
    } else {
        return skip(
            "XML_FALTA",
            " (ejecute `tessia thefactory descargar`)".into(),
        );
    };
    let params = Params {
        regimen: regimen(&rips),
        ..run.params.clone()
    };
    let prepared = match fevxml::prepare_ministerio(&xml, &params) {
        Ok(prepared) => prepared,
        Err(err @ FevxmlError::SinContrato) => {
            return skip("XML_SIN_CONTRATO", format!(" ({err})"));
        }
        Err(err) => return Err(err.into()),
    };
    let detail = prepared
        .period_error
        .as_ref()
        .map(|err| format!(" [aviso: {err}]"))
        .unwrap_or_default();
    let payload = match build_payload(&rips, &prepared.xml) {
        Ok(payload) => payload,
        Err(err) => return skip("RIPS_INVALIDO", format!(" ({err})")),
    };
    // Java refreshed the ZIP only when the adjustment did not throw.
    let refresh_zip = zip_rel.filter(|_| prepared.changed && prepared.period_error.is_none());
    Ok(Plan::Send(Box::new(SendItem {
        payload,
        write_xml: write_xml || prepared.changed,
        xml: prepared.xml,
        refresh_zip,
        detail,
    })))
}

fn zip_now() -> Option<zip::DateTime> {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    zip::DateTime::from_date_and_time(
        u16::try_from(now.year()).ok()?,
        u8::try_from(now.month()).ok()?,
        u8::try_from(now.day()).ok()?,
        u8::try_from(now.hour()).ok()?,
        u8::try_from(now.minute()).ok()?,
        u8::try_from(now.second()).ok()?,
    )
    .ok()
}

/// Writes the adjusted XML and the refreshed ZIP, posts the payload and
/// stores the response file. `None` when the run was interrupted while the
/// request failed (nothing is recorded).
///
/// The state is `DONE` when `<F>.txt` now holds a CUV, `FAILED` for
/// transport/server failures (retried) and `REVISION_MANUAL` for SISPRO
/// rejections (`_ERROR.txt`, `RECHAZADO` results, a response for another
/// invoice).
fn submit(run: &Run<'_>, target: &Target, item: SendItem) -> Result<Option<Processed>> {
    let inv = target.stem.as_str();
    let client = run.client()?;
    let mut lines = Vec::new();
    let note = if item.write_xml {
        "XML ajustado"
    } else {
        "XML sin cambios"
    };
    lines.push(format!(
        "  {inv} -> CargarFevRips (payload {} bytes, {note}){}",
        item.payload.len(),
        item.detail
    ));
    let xml_name = format!("{inv}.xml");
    if item.write_xml {
        run.publish(&xml_name, &item.xml)?;
    }
    if let Some(zip) = &item.refresh_zip {
        let refreshed = fevxml::replace_entry(&run.fetch(zip)?, &xml_name, &item.xml, zip_now())?;
        run.publish(zip, &refreshed)?;
    }
    let (status, body) = match client.cargar_fev_rips(item.payload) {
        Ok(answer) => answer,
        Err(err) => {
            if crate::shutdown::requested() {
                return Ok(None);
            }
            let token_failed = err.is_token();
            run.tally.record(token_failed);
            let msg = if token_failed {
                "Error: Token vacío"
            } else {
                "Error 500 al consumir el servicio"
            };
            let line = format!("{inv} {msg} ({err})");
            return Ok(Some(finish(lines, inv, line, Outcome::failed, true, false)));
        }
    };
    run.tally.record(false);
    let interpreted = interpret(inv, status, &body);
    let line = format!("{inv} HTTP {status} {}", interpreted.summary);
    let Some(file) = interpreted.file_name.as_deref() else {
        let failed = interpreted.failed;
        let outcome = move |reason: String| {
            if failed {
                Outcome::failed(reason)
            } else {
                Outcome::manual_review(reason)
            }
        };
        return Ok(Some(finish(lines, inv, line, outcome, true, false)));
    };
    let (content, existing_cuv) = sweep_response(client, interpreted.content.clone());
    let note = if existing_cuv.is_empty() {
        ""
    } else {
        " (RVG18: CUV existente)"
    };
    run.publish(file, &content)?;
    if file == format!("{inv}.txt") && txt_has_cuv(&content) {
        let line = format!("{line}{note} -> {file}");
        return Ok(Some(finish(
            lines,
            inv,
            line,
            |_| Outcome::done(),
            true,
            true,
        )));
    }
    // The reason shown for a rejected invoice is what SISPRO rejected it for,
    // not the HTTP status and the name of the file holding the whole response.
    let why = rejection_summary(&interpreted.response);
    let line = if why.is_empty() {
        format!("{line}{note} -> {file}")
    } else {
        format!("{inv} {why}{note}")
    };
    Ok(Some(finish(
        lines,
        inv,
        line,
        Outcome::manual_review,
        true,
        false,
    )))
}

/// The response file after the end-of-run sweep, and the CUV taken from RVG18.
fn sweep_response(client: &SisproClient, content: Vec<u8>) -> (Vec<u8>, String) {
    let lookup = |cuv: &str| client.proceso_id(cuv);
    match sweep(&content, Some(&lookup)) {
        Ok(swept) => (swept.out.map_or(content, String::into_bytes), swept.cuv),
        Err(_) => (content, String::new()),
    }
}

/// Builds the stored result: the state reason is the console line without the
/// invoice prefix.
fn finish(
    mut lines: Vec<String>,
    inv: &str,
    line: String,
    outcome: impl FnOnce(String) -> Outcome,
    submitted: bool,
    with_cuv: bool,
) -> Processed {
    let reason = line
        .strip_prefix(&format!("{inv} "))
        .unwrap_or(&line)
        .to_owned();
    lines.push(format!("  {line}"));
    Processed {
        outcome: outcome(reason),
        lines,
        submitted,
        with_cuv,
    }
}

fn process(run: &Run<'_>, target: &Target) -> Result<Option<Processed>> {
    match plan(run, target)? {
        Plan::Skip { code, detail } => Ok(Some(Processed {
            outcome: skip_outcome(code, &detail),
            lines: vec![format!("  skip {} {code}{detail}", target.stem)],
            submitted: false,
            with_cuv: false,
        })),
        Plan::Send(item) => submit(run, target, *item),
    }
}

/// Everything one worker does for a claimed invoice.
fn work(run: &Run<'_>, target: &Target, version: i64) -> Finished {
    let proceso = run.ctx.proceso();
    let give_back = || {
        let versions = HashMap::from([(target.key.clone(), version)]);
        if let Err(err) = proceso.release(Phase::Cuv, &versions, &run.previous) {
            tracing::warn!(invoice = %target.key, error = %err, "could not release the claim");
        }
    };
    if crate::shutdown::requested() {
        give_back();
        return Finished::Interrupted;
    }
    let name = lease_name(run.cargue, &target.key);
    let _lease = match run.ctx.leases().guard(&name, &run.ctx.owner, LEASE_TTL) {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            give_back();
            return Finished::OtherMachine(format!(
                "  skip {} {SKIP_OTHER_MACHINE} (otra máquina o `fev ajustar` la está procesando)",
                target.stem
            ));
        }
        Err(err) => {
            give_back();
            return Finished::Failed(err.into());
        }
    };
    let processed = match process(run, target) {
        Ok(Some(processed)) => processed,
        Ok(None) => {
            give_back();
            return Finished::Interrupted;
        }
        Err(err) => {
            give_back();
            return Finished::Failed(err);
        }
    };
    match proceso.save_outcome(Phase::Cuv, &target.key, &processed.outcome, version) {
        Ok(true) => Finished::Stored(processed),
        Ok(false) => Finished::OtherMachine(format!(
            "  skip {} {SKIP_OTHER_MACHINE} (la reclamó otra máquina; resultado descartado)",
            target.stem
        )),
        Err(err) => Finished::Failed(err.into()),
    }
}

/// The printable detail of a gate skip (Go `gate`).
fn gate_detail(code: &str, own: Option<&ProcesoState>, rips: Option<&ProcesoState>) -> String {
    let state = if code == "REVISION_MANUAL" { own } else { rips };
    let Some(state) = state else {
        return " (reprocess with --invoices)".into();
    };
    let mut detail = format!("{} {}", state.phase, state.state);
    if !state.motivo.is_empty() {
        detail.push_str(&format!(": {}", state.motivo));
    }
    format!("{detail} (reprocess with --invoices)")
}

/// Runs `cuv generar` for the invoices of `req` and returns everything it did
/// without failing on per-invoice infrastructure errors (see
/// [`Execution::errors`]). Login failures are counted in `tally`.
pub fn execute(ctx: &AppContext, req: &PhaseRequest, tally: &TokenTally) -> Result<Execution> {
    let workers = req.workers.unwrap_or(DEFAULT_WORKERS);
    if workers == 0 || workers > MAX_WORKERS {
        bail!("--workers must be between 1 and {MAX_WORKERS}");
    }
    ctx.settings
        .require(&["minsalud.numero_contrato", "company.codigo_prestador"])?;
    let artifacts = ctx.artifacts();
    let index: HashMap<String, i64> = artifacts
        .list(&req.cargue_id)?
        .into_iter()
        .map(|a| (a.rel_path, a.size))
        .collect();
    let targets = select_targets(rips_invoices(&index), req.invoices.as_deref())?;
    let keys: Vec<String> = targets.iter().map(|t| t.key.clone()).collect();
    let proceso = ctx.proceso();
    let own = proceso.load(Phase::Cuv, Some(&keys))?;
    // The gate only applies without `--invoices`: an explicit request re-sends
    // whatever the stored state.
    let gated = req.invoices.is_none();
    let rips_states = if gated {
        proceso.load(Phase::Rips, Some(&keys))?
    } else {
        HashMap::new()
    };

    let mut execution = Execution::default();
    let mut ready = Vec::new();
    for target in targets {
        let blocked = if gated {
            gate(
                own.get(&target.key).map(|s| s.state),
                rips_states.get(&target.key).map(|s| (Phase::Rips, s.state)),
            )
        } else {
            None
        };
        match blocked {
            Some(code) => {
                let detail = gate_detail(&code, own.get(&target.key), rips_states.get(&target.key));
                execution
                    .lines
                    .push(format!("  skip {} {code}: {detail}", target.stem));
                execution.report.skip(&target.key, &code);
            }
            None => ready.push(target),
        }
    }
    let items: Vec<ClaimItem> = ready
        .iter()
        .map(|t| ClaimItem::from_loaded(&t.key, own.get(&t.key)))
        .collect();
    let claimed =
        proceso.claim_available(Phase::Cuv, &items, &ctx.claim_options(&req.cargue_id))?;
    let mut mine = Vec::new();
    for target in ready {
        match claimed.get(&target.key) {
            Some(version) => mine.push((target, *version)),
            None => {
                execution
                    .lines
                    .push(format!("  skip {} {SKIP_OTHER_MACHINE}", target.stem));
                execution.report.skip(&target.key, SKIP_OTHER_MACHINE);
            }
        }
    }

    let run = Run {
        ctx,
        cargue: &req.cargue_id,
        artifacts,
        index,
        params: Params {
            codigo_prestador: ctx
                .settings
                .get("company.codigo_prestador")
                .unwrap_or_default()
                .trim()
                .to_owned(),
            contrato: ctx
                .settings
                .get("minsalud.numero_contrato")
                .unwrap_or_default()
                .to_owned(),
            regimen: String::new(),
            periodo_inicio: PERIODO_INICIO_MINISTERIO.to_owned(),
        },
        previous: own,
        tally,
        client: OnceLock::new(),
    };
    let results = par::map(mine, workers, |(target, version)| {
        let finished = work(&run, &target, version);
        (target, finished)
    });
    for (target, finished) in results {
        match finished {
            Finished::Stored(processed) => {
                execution.lines.extend(processed.lines);
                execution.submitted += usize::from(processed.submitted);
                execution.with_cuv += usize::from(processed.with_cuv);
                execution.report.push(&target.key, processed.outcome);
            }
            Finished::OtherMachine(line) => {
                execution.lines.push(line);
                execution.report.skip(&target.key, SKIP_OTHER_MACHINE);
            }
            Finished::Interrupted => {}
            Finished::Failed(err) => execution.errors.push(anyhow!("{}: {err:#}", target.stem)),
        }
    }
    Ok(execution)
}

/// The stored login-failure alert settings (`ParametrosStore`); best effort,
/// like Go: an unsaved row or any lookup error falls back to the defaults, so
/// the alert stays on rather than silent.
pub fn resolve_policy(ctx: &AppContext) -> TokenPolicy {
    match ctx.parametros().get() {
        Ok(Some(stored)) => TokenPolicy {
            percent: i64::from(stored.cuv_token_failure_percent),
            min_sample: i64::from(stored.cuv_min_sample),
        },
        _ => TokenPolicy::default(),
    }
}

/// Applies the stored policy to the tally of a whole run. Call it only after
/// every outcome was persisted: the error marks the job as failed without
/// changing any invoice's own status.
pub fn check_alert(ctx: &AppContext, tally: &TokenTally) -> Result<(), TokenFailureError> {
    check_token_failures(tally.snapshot(), resolve_policy(ctx))
}

/// Joins the per-invoice infrastructure errors.
pub fn join_errors(errors: &[anyhow::Error]) -> Option<anyhow::Error> {
    match errors {
        [] => None,
        [one] => Some(anyhow!("{one:#}")),
        many => Some(anyhow!(
            "{} facturas fallaron: {}",
            many.len(),
            many.iter()
                .map(|e| format!("{e:#}"))
                .collect::<Vec<_>>()
                .join("; ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(entries: &[(&str, i64)]) -> HashMap<String, i64> {
        entries.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    #[test]
    fn only_root_json_artifacts_named_like_invoices_are_invoices() {
        let idx = index(&[
            ("FEV10.json", 1),
            ("fev9.json", 1),
            ("AB1.json", 1),
            ("A1.json", 1),
            ("ABCDE1.json", 1),
            ("FEV1.json.bak", 1),
            ("FEV2.JSON", 1),
            ("FEV3_ERROR.txt", 1),
            ("XML/FEV4.json", 1),
            ("FEV.json", 1),
        ]);
        let stems: Vec<String> = rips_invoices(&idx).into_iter().map(|t| t.stem).collect();
        assert_eq!(stems, ["AB1", "FEV10", "fev9"]);
        let keys: Vec<String> = rips_invoices(&idx).into_iter().map(|t| t.key).collect();
        assert_eq!(keys, ["AB1", "FEV10", "FEV9"]);
    }

    #[test]
    fn requested_invoices_keep_their_order_and_unknown_ones_are_dropped() {
        let found = rips_invoices(&index(&[
            ("FEV1.json", 1),
            ("FEV2.json", 1),
            ("FEV3.json", 1),
        ]));
        let requested = vec!["FEV3".to_owned(), "NOPE1".to_owned(), "FEV1".to_owned()];
        let selected = select_targets(found.clone(), Some(&requested)).expect("selected");
        let keys: Vec<&str> = selected.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, ["FEV3", "FEV1"]);
        assert_eq!(select_targets(found, None).expect("all").len(), 3);
    }

    #[test]
    fn a_single_unknown_invoice_is_a_typed_error_several_are_a_plain_one() {
        let found = rips_invoices(&index(&[("FEV1.json", 1)]));
        let one = select_targets(found.clone(), Some(&["NOPE1".to_owned()])).expect_err("one");
        let typed = one.downcast_ref::<MissingRipsJsonError>().expect("typed");
        assert_eq!(typed.invoice, "NOPE1");
        assert_eq!(
            one.to_string(),
            "RIPS_JSON_FALTA: missing RIPS JSON for invoice NOPE1"
        );
        let two = select_targets(found, Some(&["NOPE1".to_owned(), "NOPE2".to_owned()]))
            .expect_err("two");
        assert!(two.downcast_ref::<MissingRipsJsonError>().is_none());
        assert_eq!(
            two.to_string(),
            "none of the --invoices has a <F>.json in the folder"
        );
    }

    #[test]
    fn the_zip_is_looked_up_in_xml_then_zip_and_must_not_be_empty() {
        let idx = index(&[("XML/FEV1_XML_PDF.zip", 0), ("ZIP/FEV1_XML_PDF.zip", 9)]);
        assert_eq!(
            existing_zip(&idx, "FEV1").as_deref(),
            Some("ZIP/FEV1_XML_PDF.zip")
        );
        let idx = index(&[("XML/FEV1_XML_PDF.zip", 5), ("ZIP/FEV1_XML_PDF.zip", 9)]);
        assert_eq!(
            existing_zip(&idx, "FEV1").as_deref(),
            Some("XML/FEV1_XML_PDF.zip")
        );
        assert_eq!(existing_zip(&idx, "FEV2"), None);
    }

    #[test]
    fn skip_codes_map_to_the_go_states() {
        assert_eq!(skip_outcome("YA_TIENE_CUV", ""), Outcome::done());
        assert_eq!(
            skip_outcome("XML_FALTA", " (x)"),
            Outcome::failed("XML_FALTA (x)")
        );
        assert_eq!(
            skip_outcome("XML_SIN_CONTRATO", " (y)"),
            Outcome::manual_review("XML_SIN_CONTRATO (y)")
        );
        assert_eq!(
            skip_outcome("RIPS_INVALIDO", " (z)"),
            Outcome::manual_review("RIPS_INVALIDO (z)")
        );
    }

    #[test]
    fn finish_strips_the_invoice_from_the_stored_reason() {
        let p = finish(
            vec!["  a".into()],
            "FEV1",
            "FEV1 HTTP 404 Error general".into(),
            Outcome::manual_review,
            true,
            false,
        );
        assert_eq!(p.outcome, Outcome::manual_review("HTTP 404 Error general"));
        assert_eq!(p.lines, ["  a", "  FEV1 HTTP 404 Error general"]);
    }
}
