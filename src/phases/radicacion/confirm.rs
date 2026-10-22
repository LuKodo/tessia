//! The four-step confirmation rules: what Mutual Ser reported about one
//! invoice step, classified as pending / confirmed / failed / note (Go
//! `internal/radicacion/confirm.go`). Pure: the engine gathers the evidence
//! and applies the verdict.

use std::fmt::Display;

use super::model::{FileDetail, FiltersAllItem, Load, LoadFile, Message, XmlDocument};
use super::rules::{
    crc_adjustment_note, global_rejection, initial_success_message, support_already_associated,
};
use crate::domain::radicacion::Step;

/// Result of evaluating a step against Mutual Ser evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Mutual Ser has not finished; ask again later.
    Pending,
    Confirmed,
    /// The upload finished without success; it may be retried.
    Failed,
    AdjustmentNote,
    CreditNote,
}

/// What Mutual Ser reported about one invoice step.
#[derive(Debug, Clone, Default)]
pub struct Evidence {
    pub invoice: String,
    pub nit: String,
    pub id_cargue: String,
    /// `findLoad` of `id_cargue` (`None` when there is no upload id).
    pub load: Option<Load>,
    /// `filter-xml` row (step 2).
    pub xml: Option<XmlDocument>,
    /// `filtersAll` row (steps 3 and 4).
    pub filters: Option<FiltersAllItem>,
    /// RIPS JSON findings (step 3 adjustment).
    pub details: Vec<FileDetail>,
}

/// The evaluation of a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub outcome: Outcome,
    /// `key=value` description persisted in `radicacion_estado.last_detail`;
    /// it keeps the legacy tokens other tools search for (`paso=N`,
    /// `idCargue=`, `statusType=`, `documento_soporte=`...).
    pub detail: String,
    /// The Mutual Ser message explaining a failure.
    pub message: String,
    /// The file itself was accepted (archivo cargado).
    pub file_loaded: bool,
    /// Step 3: the JSON is valid but there is no CRC support yet.
    pub support_pending: bool,
}

impl Verdict {
    /// A pending verdict carrying only a detail.
    pub fn pending(detail: &str) -> Verdict {
        Verdict {
            outcome: Outcome::Pending,
            detail: detail.to_owned(),
            message: String::new(),
            file_loaded: false,
            support_pending: false,
        }
    }
}

#[derive(Default)]
struct FileStatus {
    found: bool,
    loaded: bool,
    entry: LoadFile,
    message: String,
}

impl Load {
    /// The upload is no longer being processed.
    pub fn finished(&self) -> bool {
        !matches!(
            self.status.trim().to_uppercase().as_str(),
            "" | "EN_PROCESO" | "EN_PROGRESO"
        )
    }

    pub fn terminated(&self) -> bool {
        self.status.trim().eq_ignore_ascii_case("TERMINADO")
    }
}

fn lookup_file(load: Option<&Load>, matches: &dyn Fn(&str) -> bool, require_no_errors: bool) -> FileStatus {
    let mut status = FileStatus::default();
    let Some(load) = load else { return status };
    for file in &load.files {
        if !matches(&file.name.to_uppercase()) {
            continue;
        }
        status.found = true;
        status.entry = file.clone();
        status.loaded = file.status.eq_ignore_ascii_case("CARGADO")
            && (!require_no_errors || file.error_codes.is_empty());
        status.message = file_message(file);
        if status.loaded {
            break;
        }
    }
    status
}

/// The first ERROR description, else the first description.
fn file_message(file: &LoadFile) -> String {
    let error = file
        .messages
        .iter()
        .find(|m| m.kind.eq_ignore_ascii_case("ERROR") && !m.description.is_empty());
    let any = || file.messages.iter().find(|m| !m.description.is_empty());
    error.or_else(any).map(|m| m.description.clone()).unwrap_or_default()
}

/// The matcher and display name of the step file in `findLoad`.
fn expected_name(step: Step, invoice: &str, nit: &str) -> (Box<dyn Fn(&str) -> bool>, String) {
    let inv = invoice.to_uppercase();
    match step {
        Step::InvoiceZip => (
            Box::new(move |n: &str| {
                n == format!("{inv}_XML_PDF.ZIP")
                    || (n.contains(&inv) && n.contains("_XML_PDF") && n.ends_with(".ZIP"))
            }),
            format!("{invoice}_XML_PDF.zip"),
        ),
        Step::Cuv => (
            Box::new(move |n: &str| n == format!("{inv}.TXT")),
            format!("{invoice}.txt"),
        ),
        Step::Rips => (
            Box::new(move |n: &str| n.contains(&inv) && n.ends_with(".JSON")),
            format!("{invoice}.json"),
        ),
        Step::Crc => {
            let want = format!("CRC_{nit}_{invoice}.ZIP").to_uppercase();
            (
                Box::new(move |n: &str| n == want),
                format!("CRC_{nit}_{invoice}.zip"),
            )
        }
    }
}

struct Detail(String);

impl Detail {
    fn new(step: Step, ev: &Evidence) -> Detail {
        Detail(format!(
            "paso={} factura={} idCargue={}",
            step.as_i16(),
            ev.invoice,
            ev.id_cargue
        ))
    }

    fn add(&mut self, key: &str, value: impl Display) -> &mut Detail {
        self.0.push_str(&format!(" {key}={value}"));
        self
    }

    fn add_load(&mut self, ev: &Evidence, status: &FileStatus, expected: &str) {
        let Some(load) = &ev.load else {
            self.add("findLoad", "sin_idCargue");
            return;
        };
        self.add("estadoCargue", &load.status)
            .add("archivoEsperado", expected)
            .add("archivoEncontrado", status.found)
            .add("archivoCargado", status.loaded)
            .add("estadoArchivo", &status.entry.status)
            .add("erroresArchivo", status.entry.error_codes.join(","));
        if !status.message.is_empty() {
            self.add("mensaje", one_line(&status.message));
        }
    }

    fn add_filters(&mut self, item: Option<&FiltersAllItem>) {
        let Some(item) = item else {
            self.add("encontrado", false).add("documento_soporte", "null");
            return;
        };
        self.add("encontrado", true)
            .add("numero_factura", &item.invoice_number)
            .add("estado", &item.status)
            .add("idArchivo", &item.file_id)
            .add("idCargueRegistro", &item.id_cargue)
            .add("devuelto", item.returned || rejected_status(&item.status))
            .add("serviciosValidos", item.services_valid)
            .add("isSupportDocument", item.is_support_document);
        let Some(support) = &item.support else {
            self.add("documento_soporte", "null");
            return;
        };
        self.add("soporteActivo", support.status.eq_ignore_ascii_case("ACTIVE"));
        let raw = if support.raw.is_empty() { "{}" } else { &support.raw };
        self.add("documento_soporte", raw);
    }
}

/// Collapses whitespace runs into single spaces.
pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Classifies the evidence of one step. A global rejection turns anything that
/// is not confirmed (or already an adjustment note) into a credit note.
pub fn evaluate(step: Step, ev: &Evidence) -> Verdict {
    let mut verdict = match step {
        Step::InvoiceZip => evaluate_upload(step, ev),
        Step::Cuv => evaluate_cuv(ev),
        Step::Rips => evaluate_rips(ev),
        Step::Crc => evaluate_crc(ev),
    };
    if verdict.outcome != Outcome::Confirmed
        && verdict.outcome != Outcome::AdjustmentNote
        && (global_rejection(&verdict.detail) || global_rejection(&verdict.message))
    {
        verdict.outcome = Outcome::CreditNote;
    }
    verdict
}

fn blank_verdict(status: &FileStatus) -> Verdict {
    Verdict {
        outcome: Outcome::Pending,
        detail: String::new(),
        message: status.message.clone(),
        file_loaded: status.loaded,
        support_pending: false,
    }
}

/// Steps confirmed by `findLoad` alone (step 1).
fn evaluate_upload(step: Step, ev: &Evidence) -> Verdict {
    let (matches, expected) = expected_name(step, &ev.invoice, &ev.nit);
    let status = lookup_file(ev.load.as_ref(), &matches, true);
    let mut detail = Detail::new(step, ev);
    detail.add_load(ev, &status, &expected);
    let mut verdict = blank_verdict(&status);
    verdict.outcome = match &ev.load {
        None => Outcome::Pending,
        _ if ev.id_cargue.is_empty() => Outcome::Pending,
        Some(load) if status.found && status.loaded && load.terminated() => Outcome::Confirmed,
        Some(_) if status.found && initial_success_message(&status.message) => Outcome::Confirmed,
        Some(load) if !load.finished() => Outcome::Pending,
        Some(_) if !status.found => {
            verdict.message = "archivo no presente en el cargue".to_owned();
            Outcome::Failed
        }
        Some(_) => Outcome::Failed,
    };
    detail.add("confirmado", verdict.outcome == Outcome::Confirmed);
    verdict.detail = detail.0;
    verdict
}

/// The RF-FE-013 "CUV already associated" message for this invoice.
fn already_associated(message: &Message, invoice: &str) -> bool {
    let text = message.description.to_uppercase().replace('-', "");
    let inv = invoice.to_uppercase().replace('-', "");
    let associated = message.code.eq_ignore_ascii_case("RF-FE-013")
        || text.contains("CUV INGRESADO YA SE ENCUENTRA ASOCIADO");
    associated && !inv.is_empty() && text.contains(&inv)
}

fn evaluate_cuv(ev: &Evidence) -> Verdict {
    let (matches, expected) = expected_name(Step::Cuv, &ev.invoice, &ev.nit);
    let status = lookup_file(ev.load.as_ref(), &matches, true);
    let mut detail = Detail::new(Step::Cuv, ev);
    detail.add_load(ev, &status, &expected);
    let mut verdict = blank_verdict(&status);
    if status.found
        && status
            .entry
            .messages
            .iter()
            .any(|m| already_associated(m, &ev.invoice))
    {
        verdict.outcome = Outcome::Confirmed;
        verdict.file_loaded = true;
        detail
            .add("yaAsociado", true)
            .add("idArchivo", &status.entry.id)
            .add("confirmado", true);
        verdict.detail = detail.0;
        return verdict;
    }
    detail.add("yaAsociado", false);
    verdict.outcome = match &ev.load {
        None => Outcome::Pending,
        _ if ev.id_cargue.is_empty() => Outcome::Pending,
        Some(load) if !load.finished() => Outcome::Pending,
        Some(_) if !status.found => {
            verdict.message = "archivo no presente en el cargue".to_owned();
            Outcome::Failed
        }
        Some(load) if !status.loaded || !load.terminated() => Outcome::Failed,
        Some(_) => confirm_with_filter_xml(ev, &mut detail),
    };
    detail.add("confirmado", verdict.outcome == Outcome::Confirmed);
    verdict.detail = detail.0;
    verdict
}

/// The TXT is loaded; `filter-xml` must also show the invoice ACTIVE.
fn confirm_with_filter_xml(ev: &Evidence, detail: &mut Detail) -> Outcome {
    let Some(xml) = &ev.xml else {
        detail.add("filterXml", "encontrado=false");
        return Outcome::Pending;
    };
    detail
        .add("billId", &xml.bill_id)
        .add("status", &xml.status)
        .add("statusType", &xml.status_type)
        .add("errores", xml.error_codes.join(","));
    if xml.status.eq_ignore_ascii_case("ACTIVE") {
        Outcome::Confirmed
    } else {
        Outcome::Pending
    }
}

/// A `filtersAll` status that means the portal returned the file.
fn rejected_status(status: &str) -> bool {
    let s = status.trim().to_uppercase();
    ["RECHAZ", "DEVUELT", "ERROR", "INVALID", "NO_VALID"]
        .iter()
        .any(|marker| s.contains(marker))
}

fn evaluate_rips(ev: &Evidence) -> Verdict {
    let (matches, expected) = expected_name(Step::Rips, &ev.invoice, &ev.nit);
    let status = lookup_file(ev.load.as_ref(), &matches, true);
    let mut detail = Detail::new(Step::Rips, ev);
    if ev.load.is_some() {
        detail.add_load(ev, &status, &expected);
    }
    let mut verdict = blank_verdict(&status);
    let upload_failed = ev.load.as_ref().is_some_and(Load::finished)
        && status.found
        && !status.loaded
        && !initial_success_message(&status.message);
    if upload_failed {
        verdict.outcome = Outcome::Failed;
        detail.add("confirmado", false);
        verdict.detail = detail.0;
        return verdict;
    }
    let item = ev.filters.as_ref();
    detail.add_filters(item);
    match item {
        None => verdict.outcome = Outcome::Pending,
        Some(item) if item.returned || rejected_status(&item.status) || !item.services_valid => {
            verdict.outcome = Outcome::AdjustmentNote;
            verdict.file_loaded = true;
            detail.add("jsonValido", false).add("requiereNotaAjuste", true);
            detail.add("findDetailsByFile", findings_text(&ev.details));
        }
        Some(item) => {
            verdict.outcome = Outcome::Confirmed;
            verdict.file_loaded = true;
            verdict.support_pending = !item.is_support_document;
            detail.add("jsonValido", true);
            if verdict.support_pending {
                detail.add("soportePendiente", true);
            } else {
                detail.add("soporteYaAsociado", true);
            }
        }
    }
    detail.add("confirmado", verdict.outcome == Outcome::Confirmed);
    verdict.detail = detail.0;
    verdict
}

/// `total=N novedades=[code|type|field|detail || ...]`, at most 12 listed.
fn findings_text(details: &[FileDetail]) -> String {
    let mut findings = Vec::new();
    for (i, f) in details.iter().enumerate() {
        if i == 12 {
            findings.push(format!("+{} mas", details.len() - 12));
            break;
        }
        findings.push(format!(
            "{}|{}|{}|{}",
            f.code,
            f.kind,
            f.field,
            one_line(&f.detail)
        ));
    }
    format!("total={} novedades=[{}]", details.len(), findings.join(" || "))
}

fn evaluate_crc(ev: &Evidence) -> Verdict {
    let (matches, expected) = expected_name(Step::Crc, &ev.invoice, &ev.nit);
    let status = lookup_file(ev.load.as_ref(), &matches, false);
    let mut detail = Detail::new(Step::Crc, ev);
    detail.add_load(ev, &status, &expected);
    let loaded = status.loaded && ev.load.as_ref().is_some_and(Load::terminated);
    // The adjustment rule applies to the upload part only, like the legacy code.
    let upload_detail = detail.0.clone();
    detail.add_filters(ev.filters.as_ref());
    let mut verdict = blank_verdict(&status);
    verdict.file_loaded = loaded;
    let active = ev
        .filters
        .as_ref()
        .and_then(|f| f.support.as_ref())
        .is_some_and(|s| s.status.eq_ignore_ascii_case("ACTIVE"));
    let finished = ev.load.as_ref().is_some_and(Load::finished);
    verdict.outcome = if active {
        verdict.file_loaded = true;
        Outcome::Confirmed
    } else if support_already_associated(&status.message) {
        Outcome::AdjustmentNote
    } else if !loaded && crc_adjustment_note(&upload_detail) {
        Outcome::AdjustmentNote
    } else if finished && status.found && !status.loaded {
        Outcome::Failed
    } else if finished && !status.found {
        verdict.message = "archivo no presente en el cargue".to_owned();
        Outcome::Failed
    } else {
        Outcome::Pending
    };
    detail.add("confirmado", verdict.outcome == Outcome::Confirmed);
    verdict.detail = detail.0;
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phases::radicacion::model::SupportDocument;

    const NIT: &str = "900111222";

    fn load(status: &str, files: Vec<LoadFile>) -> Option<Load> {
        Some(Load {
            status: status.into(),
            files,
            raw: String::new(),
        })
    }

    fn err(code: &str, description: &str) -> Message {
        Message {
            kind: "ERROR".into(),
            code: code.into(),
            description: description.into(),
        }
    }

    fn file(name: &str, status: &str, messages: Vec<Message>) -> LoadFile {
        let error_codes = messages
            .iter()
            .filter(|m| m.kind.eq_ignore_ascii_case("ERROR") && !m.code.is_empty())
            .map(|m| m.code.clone())
            .collect();
        LoadFile {
            name: name.into(),
            status: status.into(),
            messages,
            error_codes,
            ..LoadFile::default()
        }
    }

    fn evidence(load: Option<Load>, id: &str) -> Evidence {
        Evidence {
            invoice: "FEV1".into(),
            nit: NIT.into(),
            id_cargue: id.into(),
            load,
            ..Evidence::default()
        }
    }

    #[test]
    fn step1_outcomes() {
        let cases: Vec<(&str, Evidence, Outcome, &str)> = vec![
            ("no upload id", evidence(None, ""), Outcome::Pending, ""),
            ("processing", evidence(load("EN_PROCESO", vec![]), "id-1"), Outcome::Pending, ""),
            ("loaded", evidence(load("TERMINADO", vec![file("FEV1_XML_PDF.zip", "CARGADO", vec![])]), "id-1"), Outcome::Confirmed, ""),
            (
                "loaded among others",
                evidence(
                    load(
                        "TERMINADO",
                        vec![
                            file("FEV2_XML_PDF.zip", "ERROR", vec![err("X", "otra")]),
                            file("x_FEV1_XML_PDF_v2.ZIP", "CARGADO", vec![]),
                        ],
                    ),
                    "id-1",
                ),
                Outcome::Confirmed,
                "",
            ),
            (
                "file error",
                evidence(load("TERMINADO", vec![file("FEV1_XML_PDF.zip", "ERROR", vec![err("E1", "La estructura del XML no cumple")])]), "id-1"),
                Outcome::Failed,
                "La estructura del XML no cumple",
            ),
            (
                "loaded with error codes",
                evidence(load("TERMINADO", vec![file("FEV1_XML_PDF.zip", "CARGADO", vec![err("E1", "bad")])]), "id-1"),
                Outcome::Failed,
                "bad",
            ),
            (
                "already validated",
                evidence(load("TERMINADO", vec![file("FEV1_XML_PDF.zip", "ERROR", vec![err("E", "ya se encuentra en estado VALIDADO")])]), "id-1"),
                Outcome::Confirmed,
                "",
            ),
            ("missing file", evidence(load("TERMINADO", vec![]), "id-1"), Outcome::Failed, "archivo no presente"),
            (
                "rejected invoice",
                evidence(load("TERMINADO", vec![file("FEV1_XML_PDF.zip", "ERROR", vec![err("E", "La factura asociada se encuentra en estado RECHAZADA")])]), "id-1"),
                Outcome::CreditNote,
                "",
            ),
        ];
        for (name, ev, want, message) in cases {
            let verdict = evaluate(Step::InvoiceZip, &ev);
            assert_eq!(verdict.outcome, want, "{name}: {}", verdict.detail);
            assert!(verdict.message.contains(message), "{name}: {}", verdict.message);
            assert!(verdict.detail.starts_with("paso=1 "), "{name}: {}", verdict.detail);
        }
    }

    #[test]
    fn step2_outcomes() {
        let active = XmlDocument {
            bill_id: "FEV1".into(),
            status: "ACTIVE".into(),
            status_type: "2".into(),
            ..XmlDocument::default()
        };
        let lagging = XmlDocument {
            status: "PENDING".into(),
            status_type: "1".into(),
            ..active.clone()
        };
        let rejected = XmlDocument {
            status: "INACTIVE".into(),
            status_type: "3".into(),
            ..active.clone()
        };
        let txt = |status: &str, messages: Vec<Message>| vec![file("FEV1.TXT", status, messages)];
        let associated = |code: &str, text: &str| Message {
            kind: "ERROR".into(),
            code: code.into(),
            description: text.into(),
        };
        let mut cases: Vec<(&str, Evidence, Outcome)> = vec![
            (
                "already associated alone is enough",
                evidence(load("EN_PROCESO", vec![file("FEV1.txt", "ERROR", vec![associated("RF-FE-013", "CUV ya asociado a FEV-1")])]), "id-2"),
                Outcome::Confirmed,
            ),
            (
                "associated message of other invoice",
                evidence(load("TERMINADO", vec![file("FEV1.txt", "ERROR", vec![associated("RF-FE-013", "asociado a FEV9")])]), "id-2"),
                Outcome::Failed,
            ),
            ("loaded + xml active", evidence(load("TERMINADO", txt("CARGADO", vec![])), "id-2"), Outcome::Confirmed),
            ("loaded, xml lagging", evidence(load("TERMINADO", txt("CARGADO", vec![])), "id-2"), Outcome::Pending),
            ("loaded, xml missing", evidence(load("TERMINADO", txt("CARGADO", vec![])), "id-2"), Outcome::Pending),
            ("xml statusType 3 is a rejection", evidence(load("TERMINADO", txt("CARGADO", vec![])), "id-2"), Outcome::CreditNote),
            ("processing", evidence(load("EN_PROCESO", txt("CARGADO", vec![])), "id-2"), Outcome::Pending),
            (
                "error",
                evidence(load("TERMINADO", txt("ERROR", vec![err("RF-FE-012", "no cuenta con una factura electrónica")])), "id-2"),
                Outcome::Failed,
            ),
        ];
        cases[2].1.xml = Some(active);
        cases[3].1.xml = Some(lagging);
        cases[5].1.xml = Some(rejected);
        for (name, ev, want) in cases {
            let verdict = evaluate(Step::Cuv, &ev);
            assert_eq!(verdict.outcome, want, "{name}: {}", verdict.detail);
        }
        let ev = evidence(load("TERMINADO", txt("ERROR", vec![err("RF-FE-012", "sin factura")])), "id");
        assert!(super::super::rules::requires_step1_reload(&evaluate(Step::Cuv, &ev).detail));
    }

    #[test]
    fn step3_outcomes() {
        let ok = FiltersAllItem {
            invoice_number: "FEV1".into(),
            status: "VALIDADO".into(),
            services_valid: true,
            services: "[..]".into(),
            ..FiltersAllItem::default()
        };
        let item = |returned, status: &str, services_valid, support| FiltersAllItem {
            invoice_number: "FEV1".into(),
            status: status.into(),
            returned,
            services_valid,
            is_support_document: support,
            ..FiltersAllItem::default()
        };
        let with = |f: Option<FiltersAllItem>| Evidence {
            filters: f,
            ..evidence(None, "")
        };
        let cases = vec![
            ("not yet listed", with(None), Outcome::Pending, false),
            ("valid, support pending", with(Some(ok)), Outcome::Confirmed, true),
            ("valid, support associated", with(Some(item(false, "", true, true))), Outcome::Confirmed, false),
            ("returned", with(Some(item(true, "", true, false))), Outcome::AdjustmentNote, false),
            ("rejected status", with(Some(item(false, "RECHAZADO", true, false))), Outcome::AdjustmentNote, false),
            ("no services", with(Some(item(false, "", false, false))), Outcome::AdjustmentNote, false),
            (
                "upload error",
                evidence(load("TERMINADO", vec![file("FEV1.json", "ERROR", vec![err("E", "Señor usuario, para continuar")])]), "id"),
                Outcome::Failed,
                false,
            ),
        ];
        for (name, ev, want, pending) in cases {
            let verdict = evaluate(Step::Rips, &ev);
            assert_eq!((verdict.outcome, verdict.support_pending), (want, pending), "{name}: {}", verdict.detail);
        }
        let mut ev = evidence(None, "");
        ev.filters = Some(item(true, "", false, false));
        ev.details = vec![FileDetail {
            code: "R1".into(),
            detail: "bad".into(),
            ..FileDetail::default()
        }];
        assert!(evaluate(Step::Rips, &ev).detail.contains("R1|"));
    }

    #[test]
    fn step4_outcomes() {
        let crc = format!("CRC_{NIT}_FEV1.zip");
        let support = |status: &str| FiltersAllItem {
            invoice_number: "FEV1".into(),
            support: Some(SupportDocument {
                status: status.into(),
                raw: format!(r#"{{"estado":"{status}"}}"#),
                ..SupportDocument::default()
            }),
            ..FiltersAllItem::default()
        };
        let with = |load: Option<Load>, id: &str, filters: Option<FiltersAllItem>| Evidence {
            filters,
            ..evidence(load, id)
        };
        let loaded = || load("TERMINADO", vec![file(&crc, "CARGADO", vec![])]);
        let cases = vec![
            ("support active", with(loaded(), "id", Some(support("ACTIVE"))), Outcome::Confirmed),
            ("support active without upload id", with(None, "", Some(support("ACTIVE"))), Outcome::Confirmed),
            ("loaded, support not yet active", with(loaded(), "id", Some(support("INACTIVE"))), Outcome::Pending),
            ("processing", with(load("EN_PROCESO", vec![]), "id", None), Outcome::Pending),
            (
                "not permitted",
                with(load("TERMINADO", vec![file(&crc, "ERROR", vec![err("E", "No permitido: no tiene factura/cuv cargada")])]), "id", None),
                Outcome::AdjustmentNote,
            ),
            (
                "support already associated",
                with(load("TERMINADO", vec![file(&crc, "ERROR", vec![err("E", "La factura ya cuenta con un soporte asociado")])]), "id", None),
                Outcome::AdjustmentNote,
            ),
            (
                "file in ERROR state is an adjustment (legacy rule)",
                with(load("TERMINADO", vec![file(&crc, "ERROR", vec![err("E", "timeout")])]), "id", None),
                Outcome::AdjustmentNote,
            ),
            (
                "other failure is retried",
                with(
                    load("TERMINADO", vec![file(&crc, "FALLIDO", vec![Message { kind: "INFO".into(), description: "timeout".into(), ..Message::default() }])]),
                    "id",
                    None,
                ),
                Outcome::Failed,
            ),
        ];
        for (name, ev, want) in cases {
            let verdict = evaluate(Step::Crc, &ev);
            assert_eq!(verdict.outcome, want, "{name}: {}", verdict.detail);
        }
        let confirmed = evaluate(Step::Crc, &with(None, "", Some(support("ACTIVE"))));
        assert!(
            confirmed.detail.contains(r#"documento_soporte={"estado":"ACTIVE"}"#),
            "{}",
            confirmed.detail
        );
        let waiting = evaluate(Step::Crc, &with(load("EN_PROCESO", vec![]), "id", None));
        assert!(waiting.detail.contains("documento_soporte=null"), "{}", waiting.detail);
    }
}
