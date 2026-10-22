//! Engine tests against a simulated Mutual Ser portal and fake files, with
//! real Postgres state (`TestDb`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rust_decimal::Decimal;

use super::api::{ApiError, ApiResult, RadApi};
use super::engine::{Config, Deps, Engine, Report, Wait};
use super::files::{Files, Found};
use super::model::{
    FileDetail, FiltersAllItem, Load, LoadFile, Message, SupportDocument, UploadFile, XmlDocument,
};
use super::timings::Timings;
use super::values::Values;
use crate::db::test_support::TestDb;
use crate::domain::radicacion::{RadState, RadStatus, Step};
use crate::stores::radicacion::RadicacionStore;
use crate::stores::valor_xml::ValorXmlStore;

const NIT: &str = "900111222";
const CARGUE: &str = "Convenio_21102026_21102026";

#[derive(Clone)]
struct Upload {
    step: Step,
    names: Vec<String>,
}

type UploadGate = Box<dyn Fn(Step, &[UploadFile]) -> Option<String> + Send + Sync>;
type LoadFn = Box<dyn Fn(&Portal, &str, &Upload) -> Load + Send + Sync>;
type XmlFn = Box<dyn Fn(&Portal, &str) -> Option<XmlDocument> + Send + Sync>;
type FiltersFn = Box<dyn Fn(&Portal, &str) -> Option<FiltersAllItem> + Send + Sync>;

/// A simulated Mutual Ser: records every call and answers like a healthy
/// portal unless a test overrides a behavior.
struct Portal {
    uploads: Mutex<HashMap<String, Upload>>,
    log: Mutex<Vec<(Step, Vec<String>)>>,
    next: AtomicUsize,
    gate: UploadGate,
    load: LoadFn,
    xml: XmlFn,
    filters: FiltersFn,
    filters_calls: AtomicUsize,
    filters_delay: Duration,
    load_calls: AtomicUsize,
}

fn name_of(step: Step, invoice: &str) -> String {
    match step {
        Step::InvoiceZip => format!("{invoice}_XML_PDF.zip"),
        Step::Cuv => format!("{invoice}.txt"),
        Step::Rips => format!("{invoice}.json"),
        Step::Crc => format!("CRC_{NIT}_{invoice}.zip"),
    }
}

fn loaded_file(name: &str) -> LoadFile {
    LoadFile {
        name: name.into(),
        status: "CARGADO".into(),
        ..LoadFile::default()
    }
}

impl Portal {
    fn happy() -> Portal {
        Portal {
            uploads: Mutex::new(HashMap::new()),
            log: Mutex::new(Vec::new()),
            next: AtomicUsize::new(0),
            gate: Box::new(|_, _| None),
            load: Box::new(|_, _, upload| Load {
                status: "TERMINADO".into(),
                files: upload.names.iter().map(|n| loaded_file(n)).collect(),
                raw: String::new(),
            }),
            xml: Box::new(|_, invoice| {
                Some(XmlDocument {
                    bill_id: invoice.into(),
                    status: "ACTIVE".into(),
                    status_type: "2".into(),
                    ..XmlDocument::default()
                })
            }),
            filters: Box::new(|portal, invoice| {
                portal.uploaded(Step::Rips, invoice).then(|| FiltersAllItem {
                    invoice_number: invoice.into(),
                    status: "VALIDADO".into(),
                    file_id: format!("f-{invoice}"),
                    services_valid: true,
                    services: "[1]".into(),
                    is_support_document: portal.uploaded(Step::Crc, invoice),
                    support: portal.uploaded(Step::Crc, invoice).then(|| SupportDocument {
                        status: "ACTIVE".into(),
                        raw: r#"{"estado":"ACTIVE"}"#.into(),
                        ..SupportDocument::default()
                    }),
                    ..FiltersAllItem::default()
                })
            }),
            filters_calls: AtomicUsize::new(0),
            filters_delay: Duration::ZERO,
            load_calls: AtomicUsize::new(0),
        }
    }

    fn uploaded(&self, step: Step, invoice: &str) -> bool {
        let name = name_of(step, invoice);
        self.log
            .lock()
            .expect("log")
            .iter()
            .any(|(s, names)| *s == step && names.contains(&name))
    }

    /// Uploads per step, in order, as `step:count` of files.
    fn upload_shape(&self) -> Vec<(i16, usize)> {
        self.log
            .lock()
            .expect("log")
            .iter()
            .map(|(s, names)| (s.as_i16(), names.len()))
            .collect()
    }

    fn steps_uploaded(&self) -> Vec<i16> {
        self.upload_shape().into_iter().map(|(s, _)| s).collect()
    }
}

impl RadApi for Portal {
    fn upload(&self, step: Step, files: &[UploadFile]) -> ApiResult<String> {
        if let Some(message) = (self.gate)(step, files) {
            return Err(ApiError::Other(message));
        }
        let id = format!("up-{}", self.next.fetch_add(1, Ordering::SeqCst) + 1);
        let names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
        self.log.lock().expect("log").push((step, names.clone()));
        self.uploads
            .lock()
            .expect("uploads")
            .insert(id.clone(), Upload { step, names });
        Ok(id)
    }

    fn find_load(&self, id: &str, _since: Option<chrono::DateTime<chrono::Utc>>) -> ApiResult<Load> {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        let upload = self.uploads.lock().expect("uploads").get(id).cloned();
        match upload {
            Some(upload) => Ok((self.load)(self, id, &upload)),
            None => Ok(Load::default()),
        }
    }

    fn filter_xml(&self, invoice: &str) -> ApiResult<Option<XmlDocument>> {
        Ok((self.xml)(self, invoice))
    }

    fn filters_all(&self, invoice: &str) -> ApiResult<Option<FiltersAllItem>> {
        self.filters_calls.fetch_add(1, Ordering::SeqCst);
        if !self.filters_delay.is_zero() {
            std::thread::sleep(self.filters_delay);
        }
        Ok((self.filters)(self, invoice))
    }

    fn file_details(&self, _file_id: &str) -> ApiResult<Vec<FileDetail>> {
        Ok(vec![FileDetail {
            code: "R1".into(),
            kind: "ERROR".into(),
            field: "edad".into(),
            detail: "valor invalido".into(),
        }])
    }
}

#[derive(Default)]
struct FakeFiles {
    missing: Mutex<HashSet<(i16, String)>>,
    xml_amount: Mutex<Option<String>>,
    finds: Mutex<Vec<(i16, String, bool)>>,
    archived: Mutex<Vec<String>>,
}

impl Files for FakeFiles {
    fn invoices(&self) -> Vec<String> {
        Vec::new()
    }

    fn find(&self, step: Step, invoice: &str, include_archived: bool) -> anyhow::Result<Found> {
        self.finds
            .lock()
            .expect("finds")
            .push((step.as_i16(), invoice.to_owned(), include_archived));
        if self
            .missing
            .lock()
            .expect("missing")
            .contains(&(step.as_i16(), invoice.to_owned()))
        {
            return Ok(Found::Missing(format!("falta {invoice}")));
        }
        let name = name_of(step, invoice);
        Ok(Found::File(UploadFile {
            invoice: invoice.into(),
            rel: format!("x/{name}"),
            path: PathBuf::from(format!("/fake/{name}")),
            name,
            size: 10,
        }))
    }

    fn archive(&self, file: &UploadFile) -> anyhow::Result<()> {
        self.archived.lock().expect("archived").push(file.name.clone());
        Ok(())
    }

    fn xml_total(&self, _file: &UploadFile) -> Option<(String, String)> {
        self.xml_amount
            .lock()
            .expect("xml")
            .clone()
            .map(|a| (a, "PayableAmount".to_owned()))
    }
}

struct Harness {
    db: TestDb,
    states: RadicacionStore,
    portal: Arc<Portal>,
    files: Arc<FakeFiles>,
    cfg: Config,
    values: Option<Arc<Values>>,
    hook: Option<Arc<dyn Fn(&str, Step) + Send + Sync>>,
    timings: Arc<Timings>,
}

fn fast_config() -> Config {
    Config {
        nit: NIT.into(),
        batch_wait: Duration::from_millis(80),
        first_poll_delay: Duration::from_millis(10),
        poll_interval: Duration::from_millis(20),
        poll_max_interval: Duration::from_millis(100),
        quick_retry_delay: Duration::from_millis(20),
        retry_interval: Duration::from_millis(100),
        reload_wait: Wait::Fixed(Duration::from_millis(10)),
        deadline: Duration::from_secs(30),
        shutdown_grace: Duration::from_secs(5),
        ..Config::default()
    }
}

impl Harness {
    fn new(portal: Portal) -> Harness {
        let db = TestDb::new();
        let states = RadicacionStore::new(db.pool());
        Harness {
            db,
            states,
            portal: Arc::new(portal),
            files: Arc::new(FakeFiles::default()),
            cfg: fast_config(),
            values: None,
            hook: None,
            timings: Arc::new(Timings::new()),
        }
    }

    fn run(&self, invoices: &[&str]) -> Report {
        let invoices: Vec<String> = invoices.iter().map(|s| (*s).to_owned()).collect();
        let mut deps = Deps::new(
            self.portal.clone(),
            self.states.clone(),
            self.files.clone(),
            self.values.clone(),
            self.timings.clone(),
        );
        deps.on_uploaded = self.hook.clone();
        Engine::new(self.cfg.clone(), deps)
            .run(CARGUE, &invoices)
            .expect("run")
    }

    fn state(&self, invoice: &str) -> RadState {
        self.states
            .load(&[invoice.to_owned()])
            .expect("load")
            .remove(invoice)
            .unwrap_or_else(|| panic!("no state for {invoice}"))
    }

    fn with_values(mut self, xml: &str, agreement: &'static str) -> Harness {
        *self.files.xml_amount.lock().expect("xml") = Some(xml.to_owned());
        self.values = Some(Arc::new(Values::new(
            ValorXmlStore::new(self.db.pool()),
            Box::new(move |_| Ok(Some(agreement.parse::<Decimal>().expect("decimal")))),
        )));
        self
    }
}

fn error_file(name: &str, code: &str, description: &str) -> LoadFile {
    LoadFile {
        name: name.into(),
        status: "ERROR".into(),
        messages: vec![Message {
            kind: "ERROR".into(),
            code: code.into(),
            description: description.into(),
        }],
        error_codes: vec![code.into()],
        ..LoadFile::default()
    }
}

#[test]
fn defaults_follow_the_documented_numbers() {
    let cfg = Config::default().resolved();
    assert_eq!((cfg.batch_max, cfg.send_workers, cfg.confirm_workers), (200, 2, 16));
    assert_eq!(cfg.prepare_concurrency, 20);
    assert_eq!(cfg.batch_wait, Duration::from_secs(2));
    assert_eq!(cfg.poll_interval, Duration::from_secs(5));
    assert_eq!(cfg.poll_max_interval, Duration::from_secs(120));
    assert_eq!((cfg.quick_retries, cfg.quick_retry_delay), (3, Duration::from_secs(3)));
    assert_eq!(cfg.retry_interval, Duration::from_secs(600));
    assert_eq!(cfg.step2_xml_polls, 5);
    assert_eq!(cfg.crc_lost_after, Duration::from_secs(90));
    assert_eq!(cfg.deadline, Duration::from_secs(3 * 3600));
    assert_eq!(Config { batch_max: 1000, ..Config::default() }.resolved().batch_max, 400);
}

#[test]
fn all_steps_are_filed_and_the_step_1_batch_shares_one_upload() {
    let h = Harness::new(Portal::happy());
    let report = h.run(&["FEV1", "FEV2", "FEV3"]);
    assert_eq!(report.done, ["FEV1", "FEV2", "FEV3"]);
    assert_eq!(report.summary(), "done=3 advanced=0 rejected=0 blocked=0 in_progress=0 unclassified=0 skipped=0");
    let shape = h.portal.upload_shape();
    assert_eq!(shape[0], (1, 3), "one id_cargue for the whole micro-batch");
    for inv in ["FEV1", "FEV2", "FEV3"] {
        let st = h.state(inv);
        assert_eq!((st.step, st.status), (Step::Crc, RadStatus::Confirmed), "{inv}");
        assert_eq!(st.folder, CARGUE);
        for step in Step::ALL {
            assert!(h.portal.uploaded(step, inv), "{inv} step {step:?}");
        }
        assert!(st.last_detail.contains(r#"documento_soporte={"estado":"ACTIVE"}"#), "{}", st.last_detail);
    }
    assert_eq!(h.files.archived.lock().expect("archived").len(), 12, "every confirmed file is archived");
    let snap = h.timings.snapshot();
    assert_eq!(snap.steps.len(), 4);
    assert_eq!(snap.steps[0].confirmed, 3);
}

#[test]
fn a_finished_invoice_is_skipped_and_reported_done_without_calls() {
    let h = Harness::new(Portal::happy());
    let mut done = RadState::new("FEV1", Step::Crc);
    done.status = RadStatus::Confirmed;
    h.states.save(&done).expect("save");
    let mut note = RadState::new("FEV2", Step::Rips);
    note.status = RadStatus::CreditNote;
    h.states.save(&note).expect("save");
    let report = h.run(&["FEV1", "FEV2"]);
    assert_eq!(report.done, ["FEV1"]);
    assert_eq!(report.skipped, ["FEV1", "FEV2"]);
    assert!(h.portal.upload_shape().is_empty());
    assert_eq!(h.portal.load_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_sent_invoice_resumes_polling_the_same_upload_and_is_never_resent() {
    let h = Harness::new(Portal::happy());
    h.portal.uploads.lock().expect("uploads").insert(
        "old-up".into(),
        Upload { step: Step::InvoiceZip, names: vec![name_of(Step::InvoiceZip, "FEV1")] },
    );
    let mut sent = RadState::new("FEV1", Step::InvoiceZip);
    sent.status = RadStatus::Sent;
    sent.upload_id = "old-up".into();
    sent.sends = 1;
    h.states.save(&sent).expect("save");
    let report = h.run(&["FEV1"]);
    assert_eq!(report.done, ["FEV1"]);
    assert_eq!(h.portal.steps_uploaded(), [2, 3, 4], "step 1 was confirmed from the stored upload id");
}

#[test]
fn a_blocked_or_rejected_invoice_is_re_evaluated() {
    let h = Harness::new(Portal::happy());
    for status in [RadStatus::Blocked, RadStatus::Rejected] {
        let mut st = RadState::new("FEV1", Step::InvoiceZip);
        st.status = status;
        st.sends = 9;
        h.states.save(&st).expect("save");
        let report = h.run(&["FEV1"]);
        assert_eq!(report.done, ["FEV1"], "{status}");
        let mut again = RadState::new("FEV1", Step::InvoiceZip);
        again.status = RadStatus::Pending;
        h.states.save(&again).expect("reset for the next round");
    }
}

#[test]
fn a_failed_batch_is_sent_file_by_file() {
    let mut portal = Portal::happy();
    portal.gate = Box::new(|step, files| {
        (step == Step::InvoiceZip && files.len() > 1).then(|| "boom".to_owned())
    });
    let h = Harness::new(portal);
    let report = h.run(&["FEV1", "FEV2"]);
    assert_eq!(report.done, ["FEV1", "FEV2"]);
    let step1: Vec<_> = h.portal.upload_shape().into_iter().filter(|(s, _)| *s == 1).collect();
    assert_eq!(step1, [(1, 1), (1, 1)]);
    let snap = h.timings.snapshot();
    assert_eq!(snap.steps[0].resends["batch_split"], 2);
}

#[test]
fn a_missing_file_blocks_the_invoice_with_the_spanish_reason() {
    let h = Harness::new(Portal::happy());
    h.files.missing.lock().expect("m").insert((1, "FEV1".into()));
    let report = h.run(&["FEV1"]);
    assert_eq!(report.blocked, ["FEV1"]);
    let st = h.state("FEV1");
    assert_eq!(st.status, RadStatus::Blocked);
    assert_eq!(
        st.last_detail,
        "No se radica ZIP XML/PDF porque no se encontro archivo para factura FEV1. falta FEV1"
    );
    assert!(h.portal.upload_shape().is_empty());
}

#[test]
fn a_missing_cuv_txt_stops_the_invoice_after_step_1() {
    let h = Harness::new(Portal::happy());
    h.files.missing.lock().expect("m").insert((2, "FEV1".into()));
    let report = h.run(&["FEV1"]);
    assert_eq!(report.blocked, ["FEV1"]);
    let st = h.state("FEV1");
    assert_eq!(st.step, Step::Cuv);
    assert_eq!(
        st.last_detail,
        "Radicacion detenida despues del paso 1: factura FEV1 no tiene TXT con CUV exitoso. falta FEV1"
    );
    assert_eq!(h.portal.steps_uploaded(), [1]);
}

#[test]
fn the_value_check_blocks_step_1_when_totals_differ() {
    let h = Harness::new(Portal::happy()).with_values("1000.00", "1500");
    let report = h.run(&["FEV1"]);
    assert_eq!(report.blocked, ["FEV1"]);
    let st = h.state("FEV1");
    assert_eq!(
        st.last_detail,
        "codigo=VALOR_FACTURA_DIFERENTE mensaje=El total del XML no coincide con el informe de convenio. valor_xml=1000 valor_convenio=1500 diferencia=-500 paso1_no_enviado=true pasos_omitidos=2,3,4"
    );
    assert!(h.portal.upload_shape().is_empty());
    let stored = ValorXmlStore::new(h.db.pool()).stored_xml_total("FEV1").expect("stored");
    assert_eq!(stored, Some((Decimal::new(100000, 2), "PayableAmount".to_owned())));
}

#[test]
fn the_value_check_lets_matching_totals_through_within_the_tolerance() {
    let h = Harness::new(Portal::happy()).with_values("1000.004", "1000.00");
    let report = h.run(&["FEV1"]);
    assert_eq!(report.done, ["FEV1"]);
}

#[test]
fn the_value_check_blocks_when_the_agreement_total_is_unavailable() {
    let mut h = Harness::new(Portal::happy());
    *h.files.xml_amount.lock().expect("xml") = Some("700.00".into());
    h.values = Some(Arc::new(Values::new(
        ValorXmlStore::new(h.db.pool()),
        Box::new(|_| Ok(None)),
    )));
    let report = h.run(&["FEV1"]);
    assert_eq!(report.blocked, ["FEV1"]);
    assert!(h.state("FEV1").last_detail.contains("codigo=VALOR_CONVENIO_NO_DISPONIBLE"));
    // A failing agreement lookup blocks as "validation unavailable".
    let mut failing = Harness::new(Portal::happy());
    *failing.files.xml_amount.lock().expect("xml") = Some("700.00".into());
    failing.values = Some(Arc::new(Values::new(
        ValorXmlStore::new(failing.db.pool()),
        Box::new(|_| Err(anyhow::anyhow!("linea ilegible"))),
    )));
    failing.run(&["FEV1"]);
    let detail = failing.state("FEV1").last_detail;
    assert!(detail.contains("codigo=VALIDACION_VALOR_NO_DISPONIBLE"), "{detail}");
    assert!(detail.contains("linea ilegible"), "{detail}");
}

#[test]
fn without_xml_total_the_stored_one_is_used() {
    let h = Harness::new(Portal::happy());
    ValorXmlStore::new(h.db.pool())
        .sync_xml_total("FEV1", Decimal::new(50000, 2), "PayableAmount", "x")
        .expect("seed");
    let mut h = h;
    h.values = Some(Arc::new(Values::new(
        ValorXmlStore::new(h.db.pool()),
        Box::new(|_| Ok(Some(Decimal::new(500, 0)))),
    )));
    assert_eq!(h.run(&["FEV1"]).done, ["FEV1"]);
    let none = Harness::new(Portal::happy());
    let mut none = none;
    none.values = Some(Arc::new(Values::new(ValorXmlStore::new(none.db.pool()), Box::new(|_| Ok(Some(Decimal::ONE))))));
    none.run(&["FEV9"]);
    assert!(none.state("FEV9").last_detail.contains("codigo=VALOR_FACTURA_NO_DISPONIBLE"));
}

#[test]
fn a_rejected_invoice_becomes_a_credit_note_and_leaves() {
    let mut portal = Portal::happy();
    portal.load = Box::new(|_, _, upload| Load {
        status: "TERMINADO".into(),
        files: vec![error_file(&upload.names[0], "E", "La factura asociada se encuentra en estado RECHAZADA")],
        raw: String::new(),
    });
    let h = Harness::new(portal);
    let report = h.run(&["FEV1"]);
    assert_eq!(report.rejected, ["FEV1"]);
    let st = h.state("FEV1");
    assert_eq!((st.step, st.status), (Step::InvoiceZip, RadStatus::CreditNote));
}

#[test]
fn a_returned_rips_json_becomes_an_adjustment_note_with_its_findings() {
    let mut portal = Portal::happy();
    portal.filters = Box::new(|portal, invoice| {
        portal.uploaded(Step::Rips, invoice).then(|| FiltersAllItem {
            invoice_number: invoice.into(),
            returned: true,
            file_id: "f1".into(),
            services_valid: true,
            ..FiltersAllItem::default()
        })
    });
    let h = Harness::new(portal);
    let report = h.run(&["FEV1"]);
    assert_eq!(report.rejected, ["FEV1"]);
    let st = h.state("FEV1");
    assert_eq!((st.step, st.status), (Step::Rips, RadStatus::AdjustmentNote));
    assert!(st.last_detail.contains("findDetailsByFile=total=1 novedades=[R1|ERROR|edad|valor invalido]"), "{}", st.last_detail);
    assert_eq!(h.portal.steps_uploaded(), [1, 2, 3]);
}

#[test]
fn max_sends_rejects_the_step_after_the_budget() {
    let mut portal = Portal::happy();
    portal.gate = Box::new(|_, _| Some("sin red".to_owned()));
    let mut h = Harness::new(portal);
    h.cfg.max_sends = 2;
    let report = h.run(&["FEV1"]);
    assert_eq!(report.rejected, ["FEV1"]);
    let st = h.state("FEV1");
    assert_eq!(st.status, RadStatus::Rejected);
    assert_eq!(st.last_detail, "paso=1 Radicacion rechazada: se agotaron 2 envios del paso ZIP XML/PDF.");
    assert_eq!(st.sends, 2);
}

#[test]
fn an_upload_error_is_retried_with_doubling_quick_delay_and_recorded() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let mut portal = Portal::happy();
    portal.gate = Box::new(move |_, _| (seen.fetch_add(1, Ordering::SeqCst) < 2).then(|| "red caida".to_owned()));
    let h = Harness::new(portal);
    let report = h.run(&["FEV1"]);
    assert_eq!(report.done, ["FEV1"]);
    let snap = h.timings.snapshot();
    assert_eq!(snap.steps[0].resends["send_error"], 2);
}

#[test]
fn step_2_losing_the_invoice_reloads_step_1_once_then_retries_step_2() {
    let step2_loads = Arc::new(AtomicUsize::new(0));
    let seen = step2_loads.clone();
    let mut portal = Portal::happy();
    portal.load = Box::new(move |_, _, upload| {
        let name = upload.names[0].clone();
        let first_failure = upload.step == Step::Cuv && seen.fetch_add(1, Ordering::SeqCst) == 0;
        Load {
            status: "TERMINADO".into(),
            files: if first_failure {
                vec![error_file(&name, "RF-FE-012", "La factura no cuenta con una factura electrónica")]
            } else {
                upload.names.iter().map(|n| loaded_file(n)).collect()
            },
            raw: String::new(),
        }
    });
    let h = Harness::new(portal);
    let report = h.run(&["FEV1"]);
    assert_eq!(report.done, ["FEV1"]);
    assert_eq!(h.portal.steps_uploaded(), [1, 2, 1, 2, 3, 4]);
    let st = h.state("FEV1");
    assert!(st.step1_reloaded && !st.reload_for_step2);
    let archived_lookups: Vec<_> = h
        .files
        .finds
        .lock()
        .expect("finds")
        .iter()
        .filter(|(s, _, archived)| *s == 1 && *archived)
        .cloned()
        .collect();
    assert!(!archived_lookups.is_empty(), "the step 1 re-send may use the filed copy");
    let snap = h.timings.snapshot();
    assert_eq!(snap.steps[1].resends["step2_reload"], 1);
}

#[test]
fn a_transient_message_is_quickly_re_sent() {
    let first = Arc::new(AtomicUsize::new(0));
    let seen = first.clone();
    let mut portal = Portal::happy();
    portal.load = Box::new(move |_, _, upload| {
        let bad = upload.step == Step::InvoiceZip && seen.fetch_add(1, Ordering::SeqCst) == 0;
        Load {
            status: "TERMINADO".into(),
            files: if bad {
                vec![error_file(&upload.names[0], "E", "Connection timed out")]
            } else {
                upload.names.iter().map(|n| loaded_file(n)).collect()
            },
            raw: String::new(),
        }
    });
    let mut h = Harness::new(portal);
    h.cfg.retry_interval = Duration::from_secs(60);
    let report = h.run(&["FEV1"]);
    assert_eq!(report.done, ["FEV1"], "a 60 s retry interval would not finish");
    assert_eq!(h.timings.snapshot().steps[0].resends["quick_retry"], 1);
}

#[test]
fn step_3_already_filed_elsewhere_is_confirmed_without_uploading() {
    let mut portal = Portal::happy();
    portal.filters = Box::new(|portal, invoice| {
        let crc = portal.uploaded(Step::Crc, invoice);
        Some(FiltersAllItem {
            invoice_number: invoice.into(),
            status: "VALIDADO".into(),
            services_valid: true,
            is_support_document: crc,
            support: crc.then(|| SupportDocument { status: "ACTIVE".into(), raw: "{}".into(), ..SupportDocument::default() }),
            ..FiltersAllItem::default()
        })
    });
    let h = Harness::new(portal);
    let mut st = RadState::new("FEV1", Step::Rips);
    st.status = RadStatus::Pending;
    h.states.save(&st).expect("save");
    let report = h.run(&["FEV1"]);
    assert_eq!(report.done, ["FEV1"]);
    assert_eq!(h.portal.steps_uploaded(), [4], "step 3 was found filed by the pre-check");
}

#[test]
fn a_slow_precheck_fails_open_and_the_file_is_uploaded() {
    let mut portal = Portal::happy();
    portal.filters_delay = Duration::from_millis(400);
    let mut h = Harness::new(portal);
    h.cfg.precheck_timeout = Wait::Fixed(Duration::from_millis(30));
    let mut st = RadState::new("FEV1", Step::Rips);
    st.status = RadStatus::Pending;
    h.states.save(&st).expect("save");
    let started = std::time::Instant::now();
    let report = h.run(&["FEV1"]);
    assert_eq!(report.done, ["FEV1"]);
    assert_eq!(h.portal.steps_uploaded(), [3, 4]);
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn the_precheck_is_skipped_for_steps_reached_in_this_run() {
    let h = Harness::new(Portal::happy());
    let report = h.run(&["FEV1"]);
    assert_eq!(report.done, ["FEV1"]);
    // Steps 3 and 4 each query filtersAll once, to confirm: no pre-checks.
    let calls = h.portal.filters_calls.load(Ordering::SeqCst);
    assert!((2..=4).contains(&calls) && calls < 4, "{calls}");
    let skipped_calls = calls;
    assert_eq!(skipped_calls, 2, "pre-checks were skipped: {skipped_calls} calls");
}

#[test]
fn the_deadline_leaves_the_upload_sent_for_the_next_run() {
    let mut portal = Portal::happy();
    portal.load = Box::new(|_, _, _| Load { status: "EN_PROCESO".into(), ..Load::default() });
    let mut h = Harness::new(portal);
    h.cfg.deadline = Duration::from_millis(400);
    let report = h.run(&["FEV1"]);
    assert!(report.deadline_reached);
    assert_eq!(report.in_progress, ["FEV1"]);
    let st = h.state("FEV1");
    assert_eq!(st.status, RadStatus::Sent);
    assert_eq!(st.upload_id, "up-1");
    assert!(st.polls >= 1);
    assert!(report.summary().ends_with("deadline_reached=true"));
}

#[test]
fn only_step_runs_one_stage_and_reports_the_advanced_invoices() {
    let h = Harness::new(Portal::happy());
    let mut at_two = RadState::new("FEV2", Step::Cuv);
    at_two.status = RadStatus::Pending;
    h.states.save(&at_two).expect("save");
    let mut h = h;
    h.cfg.only_step = Some(Step::Cuv);
    let report = h.run(&["FEV1", "FEV2"]);
    assert_eq!(report.advanced, ["FEV2"]);
    assert_eq!(report.skipped, ["FEV1"]);
    assert_eq!(h.portal.steps_uploaded(), [2]);
    let st = h.state("FEV2");
    assert_eq!((st.step, st.status), (Step::Rips, RadStatus::Pending));
}

#[test]
fn a_step_4_upload_that_never_progressed_is_re_sent() {
    let loads = Arc::new(AtomicUsize::new(0));
    let seen = loads.clone();
    let mut portal = Portal::happy();
    portal.load = Box::new(move |_, _, upload| {
        let stuck = upload.step == Step::Crc && seen.fetch_add(1, Ordering::SeqCst) == 0;
        if stuck {
            return Load {
                status: "EN_PROCESO".into(),
                files: vec![],
                raw: r#"{"estado":"EN_PROCESO","fecha":"2020-01-01T00:00:00Z"}"#.into(),
            };
        }
        Load { status: "TERMINADO".into(), files: upload.names.iter().map(|n| loaded_file(n)).collect(), raw: String::new() }
    });
    portal.filters = Box::new(|portal, invoice| {
        let crc_uploads = portal.steps_uploaded().iter().filter(|s| **s == 4).count();
        portal.uploaded(Step::Rips, invoice).then(|| FiltersAllItem {
            invoice_number: invoice.into(),
            services_valid: true,
            is_support_document: crc_uploads >= 2,
            support: (crc_uploads >= 2).then(|| SupportDocument { status: "ACTIVE".into(), raw: "{}".into(), ..SupportDocument::default() }),
            ..FiltersAllItem::default()
        })
    });
    let h = Harness::new(portal);
    let report = h.run(&["FEV1"]);
    assert_eq!(report.done, ["FEV1"]);
    assert_eq!(h.portal.steps_uploaded(), [1, 2, 3, 4, 4]);
    assert_eq!(h.timings.snapshot().steps[3].resends["crc_lost"], 1);
}

#[test]
fn the_uploaded_hook_fires_per_upload_and_a_panic_does_not_break_the_run() {
    let seen: Arc<Mutex<Vec<(String, i16)>>> = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let mut h = Harness::new(Portal::happy());
    h.hook = Some(Arc::new(move |invoice, step| {
        log.lock().expect("log").push((invoice.to_owned(), step.as_i16()));
        if step == Step::Cuv {
            panic!("hook failure");
        }
    }));
    let report = h.run(&["FEV1"]);
    assert_eq!(report.done, ["FEV1"]);
    let seen = seen.lock().expect("seen").clone();
    assert_eq!(seen.iter().map(|(_, s)| *s).collect::<Vec<_>>(), [1, 2, 3, 4]);
}

#[test]
fn batches_never_exceed_batch_max() {
    let mut h = Harness::new(Portal::happy());
    h.cfg.batch_max = 2;
    h.cfg.send_workers = 1;
    let report = h.run(&["FEV1", "FEV2", "FEV3", "FEV4", "FEV5"]);
    assert_eq!(report.done.len(), 5);
    for (step, files) in h.portal.upload_shape() {
        assert!(files <= 2, "step {step} uploaded {files} files in one batch");
    }
}

#[test]
fn duplicate_and_unnormalized_invoices_are_deduplicated() {
    let h = Harness::new(Portal::happy());
    let report = h.run(&["fev-1", "FEV1", " fev1 "]);
    assert_eq!(report.done, ["FEV1"]);
    assert_eq!(h.portal.upload_shape()[0], (1, 1));
}
