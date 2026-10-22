//! `fev ajustar --op cucon|cuota` (FEV-01..04): edits the invoice XML of a cargue
//! where it is stored and re-publishes the changed artifacts.
//!
//! Differences from the Go command, all forced by the multi-machine design:
//!
//! * the XML files are the cargue's artifacts (loose `<F>.xml`, and the `.xml`
//!   entries of `<F>.zip`, `XML/*.zip`, `ZIP/*.zip`); a changed one is published
//!   again under the same relative path;
//! * the `.tessia-cuv` file lock becomes one lease per invoice,
//!   `cuv:<cargue>:<invoice>` ([`cuv_lease_name`]), the name `cuv generar` takes
//!   too. An invoice whose lease another machine holds is skipped (Go aborted the
//!   whole command);
//! * the phase state goes through `proceso_estado` claims, so two machines never
//!   adjust the same invoice at once;
//! * a file that cannot be written marks its invoice `FAILED` and the remaining
//!   writes go on (Go stopped at the first failure and left the rest `DONE`).

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use zip::ZipArchive;

use super::xmlsrc::persist_xml_values;
use crate::context::AppContext;
use crate::domain::{Outcome, Phase, normalize_invoice, valid_invoice};
use crate::fevxml::{self, ContratoResult, MAX_ENTRY_BYTES};
use crate::phases::common::{PhaseReport, PhaseRequest};
use crate::shutdown;
use crate::storage::Artifacts;
use crate::stores::leases::LeaseGuard;
use crate::stores::proceso::ClaimItem;
use crate::util::par;

/// Go `acquirePhaseLock(.., 2*time.Hour)`.
pub const LEASE_TTL: Duration = Duration::from_secs(2 * 60 * 60);

/// Default worker threads (Go ran the targets one after another).
const DEFAULT_WORKERS: usize = 8;

/// Reason stored in `report.skipped` for an invoice another machine is working on.
pub const SKIP_OTHER_MACHINE: &str = "EN_PROCESO_OTRA_MAQUINA";

/// Name of the lease that serializes the commands that rewrite the CUV-stage
/// files of an invoice (`cuv generar`, `cuv barrer`, `fev ajustar`): Go's
/// per-invoice `.tessia-cuv` lock.
pub fn cuv_lease_name(cargue_id: &str, invoice: &str) -> String {
    format!("cuv:{cargue_id}:{invoice}")
}

/// What `fev ajustar` does to every XML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AjustarOp {
    /// NUMERO_CONTRATO 22382 becomes this CUCON (already trimmed).
    Cucon(String),
    /// PrepaidPayment `schemeID` and ANTICIPOS removal.
    Cuota,
}

impl AjustarOp {
    /// The name of the `--op` value.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Cucon(_) => "cucon",
            Self::Cuota => "cuota",
        }
    }

    /// Counters printed in the summary line, in order.
    fn counters(&self) -> &'static [&'static str] {
        match self {
            Self::Cucon(_) => &["modificados", "sin_contrato", "contrato_diferente"],
            Self::Cuota => &["procesados", "modificados", "errores"],
        }
    }
}

/// An XML file of the cargue the command works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Relative path of the artifact.
    pub rel: String,
    pub is_zip: bool,
    /// Normalized file stem (`<F>.xml`, `<F>_XML_PDF.zip`).
    pub invoice: String,
}

/// Go `fevTargets`: `<F>.xml` of the cargue root plus the `.zip` files of the
/// root, `XML/` and `ZIP/`, sorted by path. `only` (normalized invoices) keeps
/// the files of those invoices.
pub fn select_targets<'a>(
    rels: impl IntoIterator<Item = &'a str>,
    only: &HashSet<String>,
) -> Vec<Target> {
    let mut out: Vec<Target> = rels
        .into_iter()
        .filter_map(target_of)
        .filter(|t| only.is_empty() || only.contains(&t.invoice))
        .collect();
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn target_of(rel: &str) -> Option<Target> {
    let (in_root, name) = match rel.split_once('/') {
        None => (true, rel),
        Some(("XML" | "ZIP", name)) if !name.contains('/') => (false, name),
        Some(_) => return None,
    };
    let lower = name.to_ascii_lowercase();
    let is_zip = lower.ends_with(".zip");
    if !is_zip && !(in_root && lower.ends_with(".xml")) {
        return None;
    }
    let mut stem = &name[..name.len() - 4];
    if is_zip && stem.to_ascii_uppercase().ends_with("_XML_PDF") {
        stem = &stem[..stem.len() - "_XML_PDF".len()];
    }
    Some(Target {
        rel: rel.to_owned(),
        is_zip,
        invoice: normalize_invoice(stem),
    })
}

/// The outcome of editing one XML.
struct XmlEdit {
    xml: Vec<u8>,
    counter: &'static str,
    notes: Vec<String>,
}

fn edit_xml(op: &AjustarOp, xml: &[u8], name: &str) -> XmlEdit {
    match op {
        AjustarOp::Cucon(value) => {
            let (out, result) = fevxml::cucon(xml, value);
            let counter = match result {
                ContratoResult::Modificado => "modificados",
                ContratoResult::SinContrato => "sin_contrato",
                ContratoResult::ContratoDiferente => "contrato_diferente",
            };
            XmlEdit {
                xml: out,
                counter,
                notes: Vec::new(),
            }
        }
        AjustarOp::Cuota => {
            let result = fevxml::cuota_moderadora(xml, name);
            XmlEdit {
                xml: result.xml,
                counter: if result.changed {
                    "modificados"
                } else {
                    "sin_cambios"
                },
                notes: result.warnings,
            }
        }
    }
}

/// How the counter is shown next to the file name.
fn label(counter: &str) -> &str {
    match counter {
        "modificados" => "modificado",
        other => other,
    }
}

/// One XML that was edited (or inspected).
struct Item {
    /// `rel` for a loose file, `rel:entry` for a ZIP entry.
    name: String,
    counter: &'static str,
    notes: Vec<String>,
}

/// What processing one target gave.
struct TargetResult {
    invoice: String,
    rel: String,
    items: Vec<Item>,
    error: Option<String>,
    /// New bytes of the artifact when something changed.
    write: Option<Vec<u8>>,
}

fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn process_target(
    artifacts: &Artifacts,
    cargue_id: &str,
    op: &AjustarOp,
    target: Target,
) -> TargetResult {
    let mut result = TargetResult {
        invoice: target.invoice.clone(),
        rel: target.rel.clone(),
        items: Vec::new(),
        error: None,
        write: None,
    };
    let data = match artifacts.fetch_bytes(cargue_id, &target.rel) {
        Ok(data) => data,
        Err(err) => {
            result.error = Some(format!("{err:#}"));
            return result;
        }
    };
    if !target.is_zip {
        let edit = edit_xml(op, &data, base_name(&target.rel));
        if edit.xml != data {
            result.write = Some(edit.xml.clone());
        }
        result.items.push(Item {
            name: target.rel.clone(),
            counter: edit.counter,
            notes: edit.notes,
        });
        return result;
    }
    match adjust_zip(&data, op, &target.rel, &mut result.items) {
        Ok(write) => result.write = write,
        Err(err) => result.error = Some(format!("{err:#}")),
    }
    result
}

/// An `.xml` entry of a ZIP.
struct ZipXml {
    name: String,
    data: Vec<u8>,
}

fn xml_entries(zip: &[u8]) -> Result<Vec<ZipXml>> {
    let mut archive = ZipArchive::new(Cursor::new(zip))?;
    let mut out = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let name = file
            .name()
            .map(|n| n.into_owned())
            .unwrap_or_else(|_| String::from_utf8_lossy(file.name_raw()).into_owned());
        if file.is_dir() || !name.to_lowercase().ends_with(".xml") {
            continue;
        }
        let mut data = Vec::new();
        file.take(MAX_ENTRY_BYTES + 1).read_to_end(&mut data)?;
        if data.len() as u64 > MAX_ENTRY_BYTES {
            return Err(anyhow!("XML demasiado grande"));
        }
        out.push(ZipXml { name, data });
    }
    Ok(out)
}

/// Go `adjustZip`: edits every `.xml` entry and returns the new ZIP (the changed
/// entries replaced like `refrescarXmlEnZipXmlPdfSiExiste`), or `None` when
/// nothing changed. `items` receives every entry looked at, also when a later
/// one fails.
fn adjust_zip(
    zip: &[u8],
    op: &AjustarOp,
    rel: &str,
    items: &mut Vec<Item>,
) -> Result<Option<Vec<u8>>> {
    let mut changes: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in xml_entries(zip)? {
        let edit = edit_xml(op, &entry.data, base_name(&entry.name));
        items.push(Item {
            name: format!("{rel}:{}", entry.name),
            counter: edit.counter,
            notes: edit.notes,
        });
        if edit.xml != entry.data {
            changes.push((entry.name, edit.xml));
        }
    }
    if changes.is_empty() {
        return Ok(None);
    }
    let modified = zip_now();
    let mut current = zip.to_vec();
    for (name, data) in changes {
        current = fevxml::replace_entry(&current, &name, &data, modified)?;
    }
    Ok(Some(current))
}

fn zip_now() -> Option<zip::DateTime> {
    use chrono::{Datelike, Timelike};
    let now = chrono::Utc::now();
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

/// Go `fevOutcomes`: the `fev_ajuste` outcome per invoice. `DONE` with the notes
/// it fell in (`cucon: modificado`), `FAILED` when a file could not be processed
/// or written. Names that are not invoices record nothing.
#[derive(Default)]
struct Outcomes {
    order: Vec<String>,
    notes: HashMap<String, Vec<String>>,
    failed: HashSet<String>,
}

impl Outcomes {
    fn add(&mut self, invoice: &str, note: &str, failed: bool) {
        if !valid_invoice(invoice) {
            return;
        }
        let notes = self.notes.entry(invoice.to_owned()).or_insert_with(|| {
            self.order.push(invoice.to_owned());
            Vec::new()
        });
        if notes.iter().any(|n| n == note) {
            return;
        }
        notes.push(note.to_owned());
        if failed {
            self.failed.insert(invoice.to_owned());
        }
    }

    fn outcome(&self, invoice: &str) -> Outcome {
        let reason = self
            .notes
            .get(invoice)
            .map(|notes| notes.join("; "))
            .unwrap_or_default();
        if self.failed.contains(invoice) {
            Outcome::failed(reason)
        } else {
            Outcome::new(crate::domain::PhaseState::Done, reason)
        }
    }
}

/// A changed artifact to publish.
struct Write {
    rel: String,
    data: Vec<u8>,
    invoice: String,
}

/// What a run of `fev ajustar` did.
#[derive(Debug, Default)]
pub struct AjustarRun {
    /// Console lines in Go's order (per-file lines, summary, warnings).
    pub lines: Vec<String>,
    pub report: PhaseReport,
    /// Files that could not be published (the command exits non-zero).
    pub write_errors: Vec<String>,
}

/// `fev ajustar` over the cargue of `req`. Standalone command: `proceso run`
/// does not call it.
pub fn ajustar(ctx: &AppContext, req: &PhaseRequest, op: &AjustarOp) -> Result<AjustarRun> {
    let artifacts = ctx.artifacts();
    let only: HashSet<String> = req.invoices.iter().flatten().cloned().collect();
    let rows = artifacts.list(&req.cargue_id)?;
    let targets = select_targets(rows.iter().map(|r| r.rel_path.as_str()), &only);
    let results = par::map(targets, req.workers_or(DEFAULT_WORKERS), |target| {
        process_target(&artifacts, &req.cargue_id, op, target)
    });
    let mut run = AjustarRun::default();
    let mut outcomes = Outcomes::default();
    let mut writes = Vec::new();
    summarize(op, results, &mut run, &mut outcomes, &mut writes);
    apply(ctx, req, &mut run, &mut outcomes, writes)?;
    Ok(run)
}

/// Prints the per-file lines and the summary, and fills `outcomes` and `writes`.
fn summarize(
    op: &AjustarOp,
    results: Vec<TargetResult>,
    run: &mut AjustarRun,
    outcomes: &mut Outcomes,
    writes: &mut Vec<Write>,
) {
    let mut count: HashMap<&str, usize> = HashMap::new();
    for result in results {
        for item in &result.items {
            *count.entry(item.counter).or_default() += 1;
            *count.entry("procesados").or_default() += 1;
            run.lines
                .push(format!("  {} {}", item.name, label(item.counter)));
            run.lines
                .extend(item.notes.iter().map(|n| format!("    {n}")));
            let note = format!("{}: {}", op.name(), label(item.counter));
            outcomes.add(&result.invoice, &note, false);
        }
        if let Some(err) = &result.error {
            *count.entry("errores").or_default() += 1;
            *count.entry("procesados").or_default() += 1;
            run.lines.push(format!("  {} ERROR {err}", result.rel));
            let note = format!("ERROR {}: {err}", result.rel);
            outcomes.add(&result.invoice, &note, true);
        } else if let Some(data) = result.write {
            writes.push(Write {
                rel: result.rel,
                data,
                invoice: result.invoice,
            });
        }
    }
    let parts: Vec<String> = op
        .counters()
        .iter()
        .map(|c| format!("{c}={}", count.get(c).copied().unwrap_or(0)))
        .collect();
    run.lines.push(format!(
        "fev ajustar {}: {} (archivos a escribir={})",
        op.name(),
        parts.join(" "),
        writes.len()
    ));
}

/// Takes the leases, claims the invoices, publishes the changed files, stores
/// the XML totals and the outcomes.
fn apply(
    ctx: &AppContext,
    req: &PhaseRequest,
    run: &mut AjustarRun,
    outcomes: &mut Outcomes,
    writes: Vec<Write>,
) -> Result<()> {
    let (_guards, busy) = take_leases(ctx, &req.cargue_id, &writes, run)?;
    let candidates: Vec<String> = outcomes
        .order
        .iter()
        .filter(|inv| !busy.contains(*inv))
        .cloned()
        .collect();
    let claimed = claim(ctx, &req.cargue_id, &candidates, run)?;
    let mut rewritten: Vec<String> = Vec::new();
    for write in writes {
        if busy.contains(&write.invoice) {
            continue;
        }
        if !claimed.contains_key(&write.invoice) {
            run.lines.push(format!(
                "warning: skipping {} because phase claim was rejected",
                write.invoice
            ));
            continue;
        }
        publish(ctx, &req.cargue_id, write, outcomes, run, &mut rewritten);
    }
    let names = rewritten_names(&rewritten, &outcomes.failed);
    persist_xml_values(ctx, &req.cargue_id, &names);
    store_outcomes(ctx, outcomes, &claimed, run)
}

/// Leases `cuv:<cargue>:<invoice>` for the invoices with a file to write. Returns
/// the guards (released on drop) and the invoices another machine holds.
fn take_leases(
    ctx: &AppContext,
    cargue_id: &str,
    writes: &[Write],
    run: &mut AjustarRun,
) -> Result<(Vec<LeaseGuard>, HashSet<String>)> {
    let mut invoices: Vec<&str> = writes
        .iter()
        .map(|w| w.invoice.as_str())
        .filter(|inv| valid_invoice(inv))
        .collect();
    invoices.sort_unstable();
    invoices.dedup();
    let mut guards = Vec::new();
    let mut busy = HashSet::new();
    for invoice in invoices {
        let name = cuv_lease_name(cargue_id, invoice);
        match ctx.leases().guard(&name, &ctx.owner, LEASE_TTL)? {
            Some(guard) => guards.push(guard),
            None => {
                busy.insert(invoice.to_owned());
                run.lines.push(format!(
                    "aviso: {invoice} está bloqueada por otra máquina ({name}); se omite"
                ));
                run.report.skip(invoice, SKIP_OTHER_MACHINE);
            }
        }
    }
    Ok((guards, busy))
}

/// Claims `fev_ajuste` for the invoices; the ones another machine holds are
/// reported and skipped. Returns invoice -> claimed version.
fn claim(
    ctx: &AppContext,
    cargue_id: &str,
    invoices: &[String],
    run: &mut AjustarRun,
) -> Result<HashMap<String, i64>> {
    if invoices.is_empty() {
        return Ok(HashMap::new());
    }
    let store = ctx.proceso();
    let loaded = store.load(Phase::FevAjuste, Some(invoices))?;
    let items: Vec<ClaimItem> = invoices
        .iter()
        .map(|inv| ClaimItem::from_loaded(inv, loaded.get(inv)))
        .collect();
    let claimed = store.claim_available(Phase::FevAjuste, &items, &ctx.claim_options(cargue_id))?;
    for invoice in invoices.iter().filter(|inv| !claimed.contains_key(*inv)) {
        let running = loaded
            .get(invoice)
            .is_some_and(|s| s.state == crate::domain::PhaseState::EnProgreso);
        run.lines.push(if running {
            format!("warning: {invoice} is already in execution for phase fev_ajuste; skipping it")
        } else {
            format!("warning: claim rejected for {invoice} in phase fev_ajuste; skipping it")
        });
        run.report.skip(invoice, SKIP_OTHER_MACHINE);
    }
    Ok(claimed)
}

fn publish(
    ctx: &AppContext,
    cargue_id: &str,
    write: Write,
    outcomes: &mut Outcomes,
    run: &mut AjustarRun,
    rewritten: &mut Vec<String>,
) {
    let result = if shutdown::requested() {
        Err(anyhow!("interrumpido"))
    } else {
        ctx.artifacts()
            .publish_bytes(cargue_id, &write.rel, &write.data)
            .map(|_| ())
            .with_context(|| format!("write {}", base_name(&write.rel)))
    };
    match result {
        Ok(()) => rewritten.push(write.rel),
        Err(err) => {
            let note = format!("ERROR write {}: {err:#}", base_name(&write.rel));
            outcomes.add(&write.invoice, &note, true);
            run.write_errors.push(format!("{err:#}"));
        }
    }
}

/// Go `rewrittenInvoices`: the file names (without extension and `_XML_PDF`) of
/// the rewritten files whose invoice did not fail.
fn rewritten_names(rewritten: &[String], failed: &HashSet<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for rel in rewritten {
        let target = target_of(rel);
        if target.is_some_and(|t| failed.contains(&t.invoice)) {
            continue;
        }
        let file = base_name(rel);
        let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
        let name = match stem.len().checked_sub("_XML_PDF".len()) {
            Some(cut)
                if stem.is_char_boundary(cut) && stem[cut..].eq_ignore_ascii_case("_XML_PDF") =>
            {
                &stem[..cut]
            }
            _ => stem,
        };
        if seen.insert(name.to_owned()) {
            out.push(name.to_owned());
        }
    }
    out
}

fn store_outcomes(
    ctx: &AppContext,
    outcomes: &Outcomes,
    claimed: &HashMap<String, i64>,
    run: &mut AjustarRun,
) -> Result<()> {
    let store = ctx.proceso();
    for invoice in &outcomes.order {
        let Some(version) = claimed.get(invoice) else {
            continue;
        };
        let outcome = outcomes.outcome(invoice);
        if store.save_outcome(Phase::FevAjuste, invoice, &outcome, *version)? {
            run.report.push(invoice, outcome);
        } else {
            run.report.skip(invoice, SKIP_OTHER_MACHINE);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn rels(targets: &[Target]) -> Vec<&str> {
        targets.iter().map(|t| t.rel.as_str()).collect()
    }

    #[test]
    fn targets_are_root_xml_and_the_zips_of_root_xml_and_zip() {
        let all = [
            "FEV2.xml",
            "FEV1.xml",
            "FEV1.zip",
            "XML/FEV1_XML_PDF.zip",
            "XML/FEV1.xml",
            "ZIP/fev-3_xml_pdf.ZIP",
            "CRC/CRC_806_FEV1.zip",
            "derechos/FEV1.json",
            "XML/sub/FEV9_XML_PDF.zip",
            "notes.txt",
        ];
        let targets = select_targets(all, &HashSet::new());
        assert_eq!(
            rels(&targets),
            [
                "FEV1.xml",
                "FEV1.zip",
                "FEV2.xml",
                "XML/FEV1_XML_PDF.zip",
                "ZIP/fev-3_xml_pdf.ZIP",
            ]
        );
        let by_rel = |rel: &str| targets.iter().find(|t| t.rel == rel).cloned();
        assert_eq!(
            by_rel("ZIP/fev-3_xml_pdf.ZIP"),
            Some(Target {
                rel: "ZIP/fev-3_xml_pdf.ZIP".into(),
                is_zip: true,
                invoice: "FEV3".into()
            })
        );
        assert_eq!(by_rel("FEV2.xml").map(|t| t.is_zip), Some(false));
    }

    #[test]
    fn invoices_filter_matches_xml_and_zip_stems() {
        let all = ["FEV1.xml", "FEV2.xml", "XML/FEV1_XML_PDF.zip", "FEV2.zip"];
        let targets = select_targets(all, &only(&["FEV1"]));
        assert_eq!(rels(&targets), ["FEV1.xml", "XML/FEV1_XML_PDF.zip"]);
    }

    #[test]
    fn outcomes_join_distinct_notes_and_fail_sticky() {
        let mut o = Outcomes::default();
        o.add("FEV1", "cucon: modificado", false);
        o.add("FEV1", "cucon: modificado", false);
        o.add("FEV1", "cucon: sin_contrato", false);
        o.add("not-an-invoice", "x", false);
        o.add("FEV2", "ERROR a.zip: boom", true);
        o.add("FEV2", "cucon: modificado", false);
        assert_eq!(o.order, ["FEV1", "FEV2"]);
        assert_eq!(
            o.outcome("FEV1"),
            Outcome::new(
                crate::domain::PhaseState::Done,
                "cucon: modificado; cucon: sin_contrato"
            )
        );
        assert_eq!(
            o.outcome("FEV2"),
            Outcome::failed("ERROR a.zip: boom; cucon: modificado")
        );
    }

    #[test]
    fn rewritten_names_drop_the_zip_suffix_and_failed_invoices() {
        let failed: HashSet<String> = ["FEV9".to_owned()].into();
        let rels = [
            "XML/Fev-1_XML_PDF.zip".to_owned(),
            "Fev-1.xml".to_owned(),
            "FEV2.zip".to_owned(),
            "FEV9.xml".to_owned(),
        ];
        assert_eq!(rewritten_names(&rels, &failed), ["Fev-1", "FEV2"]);
    }

    #[test]
    fn lease_name_is_shared_with_cuv() {
        assert_eq!(
            cuv_lease_name("Convenio_21102026_21102026", "FEV1"),
            "cuv:Convenio_21102026_21102026:FEV1"
        );
    }
}
