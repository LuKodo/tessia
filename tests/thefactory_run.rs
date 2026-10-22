//! `thefactory descargar` end to end against mock servers, the in-memory object
//! store and a throwaway Postgres.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::json;
use tessia::config::Settings;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::{Phase, PhaseState};
use tessia::fevxml;
use tessia::http::limiter::specs;
use tessia::http::{AdaptiveLimiter, RetryPolicy};
use tessia::phases::common::{PhaseReport, PhaseRequest};
use tessia::phases::thefactory::client::{ClientConfig, TheFactoryClient};
use tessia::phases::thefactory::cpscan::{
    Acquirer, CpscanApi, CpscanFiles, CpscanHttp, CpscanSidecar, CpscanTransport,
};
use tessia::phases::thefactory::{self, Runtime, TfInput, run_inputs};
use tessia::stores::proceso::ClaimItem;
use tessia::testing::{MockResponse, MockServer};

const CARGUE: &str = "Convenio_21102026_21102026";

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// A downloadable XML with a COPAGO block and a prepaid payment to clean.
fn raw_xml(invoice: &str) -> Vec<u8> {
    format!(
        "<Invoice><ID>{invoice}</ID><cac:PrepaidPayment><cbc:ID>1</cbc:ID>\
         <cbc:InstructionID>COPAGO</cbc:InstructionID></cac:PrepaidPayment>\n  \
         <AdditionalInformation><Name>COPAGO</Name><Value>10</Value></AdditionalInformation>\
         </Invoice>"
    )
    .into_bytes()
}

fn pdf_bytes() -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    pdf.resize(3000, b'x');
    pdf
}

fn jpg() -> Vec<u8> {
    let mut b = b"\xff\xd8\xff\xe0".to_vec();
    b.extend_from_slice(&[9; 64]);
    b
}

type TfHandler = dyn Fn(&str, &str) -> MockResponse + Send + Sync;

/// The Factory mock: `handler(kind, invoice)` with kind `xml` or `pdf`.
fn tf_server(handler: Arc<TfHandler>) -> MockServer {
    MockServer::start(move |req| {
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
        let invoice = body["documento"].as_str().unwrap_or_default().to_owned();
        let kind = if req.path.ends_with("GenerarContenedor") {
            "xml"
        } else {
            "pdf"
        };
        handler(kind, &invoice)
    })
}

fn good(kind: &str, invoice: &str) -> MockResponse {
    match kind {
        "xml" => MockResponse::json(
            200,
            &json!({ "ContenedorXml": b64(&raw_xml(invoice)) }).to_string(),
        ),
        _ => MockResponse::json(200, &json!({ "documento": b64(&pdf_bytes()) }).to_string()),
    }
}

fn good_tf() -> MockServer {
    tf_server(Arc::new(good))
}

/// CPScan API: `origen` per SSC (404 when unknown).
fn api_server(origins: &[(&str, &str)]) -> MockServer {
    let origins: HashMap<String, String> = origins
        .iter()
        .map(|(ssc, origin)| ((*ssc).to_owned(), (*origin).to_owned()))
        .collect();
    MockServer::start(move |req| {
        let ssc = req.path.trim_start_matches("/documentos/");
        match origins.get(ssc) {
            Some(origin) => {
                MockResponse::json(200, &json!({ "ssc": ssc, "origen": origin }).to_string())
            }
            None => MockResponse::text(404, ""),
        }
    })
}

/// CPScan bucket: serves `formula-<ssc>.jpg` for the given SSC values.
fn bucket_server(with_formula: &[&str]) -> MockServer {
    let names: HashSet<String> = with_formula
        .iter()
        .map(|ssc| format!("/files/formula-{ssc}.jpg/download"))
        .collect();
    MockServer::start(move |req| {
        if names.contains(&req.path) {
            MockResponse::bytes(200, jpg())
        } else {
            MockResponse::text(404, "")
        }
    })
}

struct Limiters {
    thefactory: Arc<AdaptiveLimiter>,
    cpscan: Arc<AdaptiveLimiter>,
}

fn runtime(tf: &MockServer, api: &MockServer, bucket: &MockServer) -> (Runtime, Limiters) {
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");
    let limiters = Limiters {
        thefactory: Arc::new(AdaptiveLimiter::new(specs::THEFACTORY.options())),
        cpscan: Arc::new(AdaptiveLimiter::new(specs::CPSCAN.options())),
    };
    let thefactory = TheFactoryClient::new(ClientConfig {
        base_url: tf.url().to_owned(),
        token_empresa: "emp".into(),
        token_password: "pw".into(),
        http: http.clone(),
        limiter: Some(limiters.thefactory.clone()),
        sleeper: Some(Arc::new(|_| {})),
    })
    .expect("thefactory client");
    let transport = Arc::new(CpscanTransport::new(
        CpscanHttp::plain(http),
        Some(limiters.cpscan.clone()),
        RetryPolicy::instant(3),
    ));
    let cpscan = Acquirer::new(
        CpscanApi::new(api.url(), transport.clone()).expect("api"),
        CpscanFiles::new(&format!("{}/files/", bucket.url()), transport).expect("files"),
    );
    (Runtime { thefactory, cpscan }, limiters)
}

fn run(
    ctx: &AppContext,
    inputs: Vec<TfInput>,
    servers: (&MockServer, &MockServer, &MockServer),
) -> PhaseReport {
    let (runtime, limiters) = runtime(servers.0, servers.1, servers.2);
    let report = run_inputs(ctx, CARGUE, inputs, 4, Some(runtime)).expect("run");
    assert_eq!(limiters.thefactory.active(), 0, "no slot left held");
    assert_eq!(limiters.cpscan.active(), 0, "no slot left held");
    report
}

fn state(ctx: &AppContext, invoice: &str) -> (PhaseState, String, i32) {
    let loaded = ctx
        .proceso()
        .load(Phase::TheFactory, Some(&[invoice.to_owned()]))
        .expect("load");
    let row = loaded.get(invoice).expect("state row");
    (row.state, row.motivo.clone(), row.intentos)
}

fn keys(ctx: &AppContext) -> Vec<String> {
    ctx.artifacts()
        .list(CARGUE)
        .expect("list")
        .into_iter()
        .map(|a| a.rel_path)
        .collect()
}

fn fetch(ctx: &AppContext, rel: &str) -> Vec<u8> {
    ctx.artifacts().fetch_bytes(CARGUE, rel).expect(rel)
}

fn count(server: &MockServer, needle: &str) -> usize {
    server
        .requests()
        .iter()
        .filter(|r| r.path.contains(needle))
        .count()
}

fn world() -> (TestDb, AppContext) {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    (db, ctx)
}

#[test]
fn downloads_cleans_zips_publishes_and_records_done() {
    let (_db, ctx) = world();
    let (tf, api, bucket) = (
        good_tf(),
        api_server(&[("100", "APP")]),
        bucket_server(&["100"]),
    );
    let report = run(
        &ctx,
        vec![
            TfInput::new("FEV1", Some("100")),
            TfInput::new("FEV2", None),
        ],
        (&tf, &api, &bucket),
    );

    assert_eq!(report.outcomes.len(), 2);
    assert!(report.skipped.is_empty());
    assert_eq!(
        keys(&ctx),
        [
            "CPSCAN/FEV1.cpscan.json",
            "CPSCAN/FEV1.support.jpg",
            "FEV1.pdf",
            "FEV1.xml",
            "FEV2.pdf",
            "FEV2.xml",
            "XML/FEV1_XML_PDF.zip",
            "XML/FEV2_XML_PDF.zip",
        ]
    );
    // The XML is the cleaned download (TF-06).
    let xml = fetch(&ctx, "FEV1.xml");
    assert_eq!(xml, fevxml::normalize_download(&raw_xml("FEV1")));
    let text = String::from_utf8(xml.clone()).expect("utf8");
    assert!(
        text.contains(r#"<cbc:ID schemeID="01">1</cbc:ID>"#),
        "{text}"
    );
    assert!(!text.contains("AdditionalInformation"), "{text}");
    assert_eq!(fetch(&ctx, "FEV1.pdf"), pdf_bytes());
    // The ZIP holds the PDF first, then the XML.
    let zip_bytes = fetch(&ctx, "XML/FEV1_XML_PDF.zip");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).expect("zip");
    let names: Vec<String> = (0..archive.len())
        .map(|i| {
            archive
                .by_index(i)
                .expect("entry")
                .name()
                .expect("name")
                .into_owned()
        })
        .collect();
    assert_eq!(names, ["FEV1.pdf", "FEV1.xml"]);
    let mut entry = String::new();
    archive
        .by_name("FEV1.xml")
        .expect("xml entry")
        .read_to_string(&mut entry)
        .expect("read");
    assert_eq!(entry.as_bytes(), xml.as_slice());
    // The sidecar documents the lookup; the payload is the downloaded file.
    let sidecar = CpscanSidecar::parse(&fetch(&ctx, "CPSCAN/FEV1.cpscan.json")).expect("sidecar");
    assert_eq!(sidecar.ssc, "100");
    assert!(sidecar.found);
    assert_eq!(
        (sidecar.origin.as_str(), sidecar.file.as_str()),
        ("APP", "formula-100.jpg")
    );
    assert_eq!(sidecar.ext, "jpg");
    assert_eq!(fetch(&ctx, "CPSCAN/FEV1.support.jpg"), jpg());
    // State and requests.
    for invoice in ["FEV1", "FEV2"] {
        assert_eq!(state(&ctx, invoice), (PhaseState::Done, String::new(), 1));
    }
    let first = &tf.requests()[0];
    assert_eq!(first.method, "POST");
    assert_eq!(
        (first.path.as_str(), first.body_text().as_str()),
        (
            "/api/archivos/GenerarContenedor",
            r#"{"tokenEmpresa":"emp","tokenPassword":"pw","documento":"FEV1"}"#
        )
    );
    assert_eq!(count(&api, "/documentos/"), 1, "FEV2 has no SSC");
    // The XML total tracking row exists (best effort side effect).
    let values = ctx
        .valor_xml()
        .stored_xml_values(&["FEV1".to_owned()])
        .expect("values");
    assert!(values.contains_key("FEV1"));
    // The private workdirs are gone.
    let leftovers = std::fs::read_dir(&ctx.workdir_root).expect("root").count();
    assert_eq!(leftovers, 0, "workdirs are deleted after use");
}

#[test]
fn complete_invoices_are_never_downloaded_again() {
    let (_db, ctx) = world();
    let (tf, api, bucket) = (
        good_tf(),
        api_server(&[("100", "APP")]),
        bucket_server(&["100"]),
    );
    let inputs = || {
        vec![
            TfInput::new("FEV1", Some("100")),
            TfInput::new("FEV2", None),
        ]
    };
    run(&ctx, inputs(), (&tf, &api, &bucket));
    let before = (
        tf.requests().len(),
        api.requests().len(),
        bucket.requests().len(),
    );

    let report = run(&ctx, inputs(), (&tf, &api, &bucket));
    assert_eq!(
        (
            tf.requests().len(),
            api.requests().len(),
            bucket.requests().len()
        ),
        before,
        "YA_EXISTE: no request at all"
    );
    assert_eq!(report.outcomes.len(), 2, "every invoice is recorded again");
    assert_eq!(state(&ctx, "FEV1"), (PhaseState::Done, String::new(), 2));
}

#[test]
fn a_missing_document_fails_the_invoice_and_a_rerun_fetches_only_that_part() {
    let (_db, ctx) = world();
    let xml_ready = Arc::new(AtomicBool::new(false));
    let flag = xml_ready.clone();
    let tf = tf_server(Arc::new(move |kind, invoice| {
        if kind == "xml" && !flag.load(Ordering::SeqCst) {
            MockResponse::json(404, "")
        } else {
            good(kind, invoice)
        }
    }));
    let (api, bucket) = (api_server(&[]), bucket_server(&[]));
    let report = run(&ctx, vec![TfInput::new("FEV3", None)], (&tf, &api, &bucket));

    assert!(report.has_failures());
    assert_eq!(
        state(&ctx, "FEV3"),
        (
            PhaseState::Failed,
            "XML=NO_DISPONIBLE PDF=DESCARGADO ZIP=FALTA (XML: HTTP 404 )".to_owned(),
            1
        )
    );
    assert_eq!(
        keys(&ctx),
        ["FEV3.pdf"],
        "only what was downloaded is published"
    );

    xml_ready.store(true, Ordering::SeqCst);
    let pdf_requests = count(&tf, "DescargaPDF");
    run(&ctx, vec![TfInput::new("FEV3", None)], (&tf, &api, &bucket));
    assert_eq!(count(&tf, "DescargaPDF"), pdf_requests, "the PDF is kept");
    assert_eq!(state(&ctx, "FEV3"), (PhaseState::Done, String::new(), 2));
    assert_eq!(keys(&ctx), ["FEV3.pdf", "FEV3.xml", "XML/FEV3_XML_PDF.zip"]);
}

#[test]
fn invalid_downloads_are_reported_with_the_java_codes() {
    let (_db, ctx) = world();
    let tf = tf_server(Arc::new(|kind, _| match kind {
        "xml" => MockResponse::json(200, &json!({ "ContenedorXml": b64(b"<a/>") }).to_string()),
        _ => MockResponse::json(200, &json!({ "documento": b64(b"%PDF-1.4") }).to_string()),
    }));
    let (api, bucket) = (api_server(&[]), bucket_server(&[]));
    run(&ctx, vec![TfInput::new("FEV4", None)], (&tf, &api, &bucket));
    assert_eq!(
        state(&ctx, "FEV4"),
        (
            PhaseState::Failed,
            "XML=ERROR PDF=PDF_INVALIDO ZIP=FALTA (fevxml: XML de 4 bytes (< 40); \
             invoicefs: no es un PDF: 8 bytes (< 1024))"
                .to_owned(),
            1
        )
    );
    assert!(keys(&ctx).is_empty());
}

#[test]
fn cpscan_with_nothing_is_recorded_and_never_repeated() {
    let (_db, ctx) = world();
    let (tf, api, bucket) = (good_tf(), api_server(&[("7", "PC")]), bucket_server(&[]));
    run(
        &ctx,
        vec![TfInput::new("FEV5", Some("7"))],
        (&tf, &api, &bucket),
    );

    let sidecar = CpscanSidecar::parse(&fetch(&ctx, "CPSCAN/FEV5.cpscan.json")).expect("sidecar");
    assert_eq!((sidecar.found, sidecar.file.as_str()), (true, ""));
    assert!(!keys(&ctx).iter().any(|k| k.contains(".support.")));
    assert_eq!(
        state(&ctx, "FEV5"),
        (
            PhaseState::Done,
            "CPSCAN sin soporte (SSC 7, origen PC)".to_owned(),
            1
        )
    );

    let before = api.requests().len();
    run(
        &ctx,
        vec![TfInput::new("FEV5", Some("7"))],
        (&tf, &api, &bucket),
    );
    assert_eq!(
        api.requests().len(),
        before,
        "a settled lookup is not repeated"
    );
    assert_eq!(
        state(&ctx, "FEV5").1,
        "CPSCAN sin soporte (SSC 7, origen PC)",
        "the warning survives the re-run"
    );
}

#[test]
fn cpscan_transient_failure_writes_nothing_and_the_rerun_fetches_only_cpscan() {
    let (_db, ctx) = world();
    let up = Arc::new(AtomicBool::new(false));
    let flag = up.clone();
    let api = MockServer::start(move |_| {
        if flag.load(Ordering::SeqCst) {
            MockResponse::json(200, r#"{"origen":"APP"}"#)
        } else {
            MockResponse::text(503, "")
        }
    });
    let (tf, bucket) = (good_tf(), bucket_server(&["8"]));
    run(
        &ctx,
        vec![TfInput::new("FEV6", Some("8"))],
        (&tf, &api, &bucket),
    );

    let (st, motivo, _) = state(&ctx, "FEV6");
    assert_eq!(st, PhaseState::Done, "CPScan never changes the outcome");
    assert!(motivo.starts_with("CPSCAN pendiente (SSC 8): "), "{motivo}");
    assert!(
        !keys(&ctx).iter().any(|k| k.starts_with("CPSCAN/")),
        "nothing written"
    );
    assert_eq!(api.requests().len(), 3, "three attempts");

    up.store(true, Ordering::SeqCst);
    let tf_requests = tf.requests().len();
    run(
        &ctx,
        vec![TfInput::new("FEV6", Some("8"))],
        (&tf, &api, &bucket),
    );
    assert_eq!(
        tf.requests().len(),
        tf_requests,
        "XML/PDF/ZIP are not requested again"
    );
    assert_eq!(state(&ctx, "FEV6").1, "");
    assert!(keys(&ctx).contains(&"CPSCAN/FEV6.cpscan.json".to_owned()));
    assert!(keys(&ctx).contains(&"CPSCAN/FEV6.support.jpg".to_owned()));
}

#[test]
fn a_payload_less_sidecar_is_pending_again() {
    let (_db, ctx) = world();
    let (tf, api, bucket) = (
        good_tf(),
        api_server(&[("9", "APP")]),
        bucket_server(&["9"]),
    );
    run(
        &ctx,
        vec![TfInput::new("FEV7", Some("9"))],
        (&tf, &api, &bucket),
    );
    ctx.artifacts()
        .delete(CARGUE, "CPSCAN/FEV7.support.jpg")
        .expect("delete payload");

    let before = api.requests().len();
    run(
        &ctx,
        vec![TfInput::new("FEV7", Some("9"))],
        (&tf, &api, &bucket),
    );
    assert_eq!(api.requests().len(), before + 1, "the lookup is repeated");
    assert!(keys(&ctx).contains(&"CPSCAN/FEV7.support.jpg".to_owned()));
}

#[test]
fn cpscan_and_the_xml_download_run_at_the_same_time() {
    let (_db, ctx) = world();
    let api_seen = Arc::new(AtomicBool::new(false));
    let tf_seen = Arc::new(AtomicBool::new(false));
    let waited_ok = Arc::new(AtomicBool::new(false));
    let (a, t, w) = (api_seen.clone(), tf_seen.clone(), waited_ok.clone());
    let tf = tf_server(Arc::new(move |kind, invoice| {
        t.store(true, Ordering::SeqCst);
        if kind == "xml" {
            let start = Instant::now();
            while !a.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(10));
            }
            w.store(a.load(Ordering::SeqCst), Ordering::SeqCst);
        }
        good(kind, invoice)
    }));
    let (a, t) = (api_seen.clone(), tf_seen.clone());
    let api = MockServer::start(move |_| {
        let start = Instant::now();
        while !t.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        a.store(true, Ordering::SeqCst);
        MockResponse::json(200, r#"{"origen":"APP"}"#)
    });
    let bucket = bucket_server(&["10"]);
    run(
        &ctx,
        vec![TfInput::new("FEV8", Some("10"))],
        (&tf, &api, &bucket),
    );
    assert!(
        waited_ok.load(Ordering::SeqCst),
        "the lookup was in flight during the XML download"
    );
}

#[test]
fn invoices_owned_by_another_machine_are_skipped() {
    let (_db, ctx) = world();
    let other = ctx.clone().with_owner("otra-maquina:1:abc");
    other
        .proceso()
        .claim_available(
            Phase::TheFactory,
            &[ClaimItem::new("FEV9")],
            &other.claim_options(CARGUE),
        )
        .expect("other machine claims");
    let (tf, api, bucket) = (good_tf(), api_server(&[]), bucket_server(&[]));
    let report = run(
        &ctx,
        vec![TfInput::new("FEV9", None), TfInput::new("FEV10", None)],
        (&tf, &api, &bucket),
    );
    assert_eq!(
        report.skipped,
        [("FEV9".to_owned(), "EN_PROCESO_OTRA_MAQUINA".to_owned())]
    );
    assert_eq!(report.outcomes.len(), 1);
    assert!(
        tf.requests()
            .iter()
            .all(|r| !r.body_text().contains("FEV9"))
    );
    assert_eq!(state(&ctx, "FEV9").0, PhaseState::EnProgreso);
    assert_eq!(state(&ctx, "FEV10").0, PhaseState::Done);
}

#[test]
fn run_phase_validates_workers_and_required_settings_before_any_side_effect() {
    let (_db, ctx) = world();
    let mut request = PhaseRequest::new(CARGUE);
    request.invoices = Some(vec!["FEV1".into()]);

    request.workers = Some(31);
    let err = thefactory::run_phase(&ctx, &request).expect_err("31 workers");
    assert!(
        err.to_string()
            .contains("--workers debe estar entre 1 y 30"),
        "{err}"
    );

    request.workers = Some(30);
    let err = thefactory::run_phase(&ctx, &request).expect_err("no settings");
    let message = format!("{err:#}");
    for env in [
        "TESSIA_THEFACTORY_BASE_URL",
        "TESSIA_THEFACTORY_TOKEN_PASSWORD",
        "TESSIA_CPSCAN_BUCKET_URL",
    ] {
        assert!(message.contains(env), "{message}");
    }
    let rows = ctx.proceso().load(Phase::TheFactory, None).expect("load");
    assert!(rows.is_empty(), "nothing was claimed");

    let settings = Settings::from_pairs([("TESSIA_THEFACTORY_BASE_URL", "x")]);
    assert!(Runtime::from_settings(&ctx.clone().with_settings(settings)).is_err());
}

#[test]
fn run_phase_builds_its_clients_from_the_settings() {
    let (_db, ctx) = world();
    let (tf, api, bucket) = (good_tf(), api_server(&[]), bucket_server(&[]));
    let settings = Settings::default()
        .with("thefactory.base_url", tf.url())
        .with("thefactory.token_empresa", "emp")
        .with("thefactory.token_password", "pw")
        .with("cpscan.base_url", api.url())
        .with("cpscan.bucket_url", &format!("{}/files/", bucket.url()));
    let ctx = ctx.with_settings(settings);
    let mut request = PhaseRequest::new(CARGUE);
    request.invoices = Some(vec!["FEV1".into()]);
    request.workers = Some(2);
    let report = thefactory::run_phase(&ctx, &request).expect("run_phase");
    assert_eq!(report.counters().done, 1);
    assert_eq!(state(&ctx, "FEV1").0, PhaseState::Done);
    assert!(keys(&ctx).contains(&"XML/FEV1_XML_PDF.zip".to_owned()));
}

#[test]
fn invalid_invoice_numbers_are_rejected_before_any_work() {
    let (_db, ctx) = world();
    let (tf, api, bucket) = (good_tf(), api_server(&[]), bucket_server(&[]));
    let (runtime, _) = runtime(&tf, &api, &bucket);
    let err = run_inputs(
        &ctx,
        CARGUE,
        vec![TfInput::new("../x", None)],
        1,
        Some(runtime),
    )
    .expect_err("invalid");
    assert!(err.to_string().contains("factura inválida"), "{err}");
    assert!(tf.requests().is_empty());
}

#[test]
fn unavailable_documents_do_not_stop_the_other_invoices() {
    let (_db, ctx) = world();
    let tf = tf_server(Arc::new(|kind, invoice| {
        if invoice == "FEV11" {
            MockResponse::json(200, r#"{"mensaje":"Documento no disponible"}"#)
        } else {
            good(kind, invoice)
        }
    }));
    let (api, bucket) = (api_server(&[]), bucket_server(&[]));
    let report = run(
        &ctx,
        vec![TfInput::new("FEV11", None), TfInput::new("FEV12", None)],
        (&tf, &api, &bucket),
    );
    let counters = report.counters();
    assert_eq!((counters.done, counters.failed), (1, 1));
    assert_eq!(
        state(&ctx, "FEV11").1,
        "XML=NO_DISPONIBLE PDF=NO_DISPONIBLE ZIP=FALTA (XML: HTTP 200 Documento no disponible; \
         PDF: HTTP 200 Documento no disponible)"
    );
}

#[test]
fn files_published_by_the_older_layout_count_as_complete() {
    let (_db, ctx) = world();
    let artifacts = ctx.artifacts();
    artifacts
        .publish_bytes(CARGUE, "FEV13.xml", b"<a>padding</a>")
        .expect("xml");
    artifacts
        .publish_bytes(CARGUE, "FEV13.pdf", &pdf_bytes())
        .expect("pdf");
    artifacts
        .publish_bytes(CARGUE, "ZIP/FEV13_XML_PDF.zip", b"PK-old")
        .expect("zip");
    let (tf, api, bucket) = (good_tf(), api_server(&[]), bucket_server(&[]));
    let report = run(
        &ctx,
        vec![TfInput::new("FEV13", None)],
        (&tf, &api, &bucket),
    );
    assert!(tf.requests().is_empty());
    assert_eq!(report.counters().done, 1);

    // A PDF of exactly 1024 bytes is not "present" (> 1024 is required).
    artifacts
        .publish_bytes(CARGUE, "FEV14.xml", b"<a>padding</a>")
        .expect("xml");
    artifacts
        .publish_bytes(CARGUE, "FEV14.pdf", &vec![b'x'; 1024])
        .expect("pdf");
    artifacts
        .publish_bytes(CARGUE, "XML/FEV14_XML_PDF.zip", b"PK")
        .expect("zip");
    run(
        &ctx,
        vec![TfInput::new("FEV14", None)],
        (&tf, &api, &bucket),
    );
    assert_eq!(count(&tf, "DescargaPDF"), 1);
    assert_eq!(count(&tf, "GenerarContenedor"), 0);
}
