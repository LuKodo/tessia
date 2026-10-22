//! `thefactory descargar` reading the invoices and SSC from the dataset store.

use std::collections::HashMap;

use base64::Engine;
use serde_json::json;
use tessia::config::Settings;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::dataset::test_support::{DatasetBuilder, InvoiceBuilder, sample_line};
use tessia::domain::{Phase, PhaseState};
use tessia::phases::common::PhaseRequest;
use tessia::phases::thefactory;
use tessia::stores::dataset::test_support::seed_cargue;
use tessia::testing::{MockResponse, MockServer};

const LOAD: &str = "Convenio_21102026_21102026";

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn jpg() -> Vec<u8> {
    let mut b = b"\xff\xd8\xff\xe0".to_vec();
    b.extend_from_slice(&[9; 64]);
    b
}

struct Servers {
    tf: MockServer,
    api: MockServer,
    bucket: MockServer,
}

fn servers() -> Servers {
    let tf = MockServer::start(|req| {
        if req.path.ends_with("GenerarContenedor") {
            let xml = b"<Invoice><ID>1</ID><x>padding padding padding padding</x></Invoice>";
            MockResponse::json(200, &json!({ "ContenedorXml": b64(xml) }).to_string())
        } else {
            let mut pdf = b"%PDF-1.4\n".to_vec();
            pdf.resize(2000, b'x');
            MockResponse::json(200, &json!({ "documento": b64(&pdf) }).to_string())
        }
    });
    let api = MockServer::start(|req| match req.path.as_str() {
        "/documentos/100" => MockResponse::json(200, r#"{"origen":"APP"}"#),
        _ => MockResponse::text(404, ""),
    });
    let bucket = MockServer::start(|req| {
        if req.path == "/files/formula-100.jpg/download" {
            MockResponse::bytes(200, jpg())
        } else {
            MockResponse::text(404, "")
        }
    });
    Servers { tf, api, bucket }
}

fn context(db: &TestDb, s: &Servers) -> AppContext {
    let settings = Settings::default()
        .with("thefactory.base_url", s.tf.url())
        .with("thefactory.token_empresa", "emp")
        .with("thefactory.token_password", "pw")
        .with("cpscan.base_url", s.api.url())
        .with("cpscan.bucket_url", &format!("{}/files/", s.bucket.url()));
    AppContext::for_tests(db).with_settings(settings)
}

fn seed_dataset(db: &TestDb, ctx: &AppContext) {
    let seq = seed_cargue(&db.pool(), LOAD, 'a');
    let mut with_ssc = sample_line("FEV1", 1, "10");
    with_ssc.ssc = "SSC 100".into();
    let mut other_ssc = sample_line("FEV1", 2, "10");
    other_ssc.ssc = "555".into();
    let ds = DatasetBuilder::new(seq)
        .invoice(
            InvoiceBuilder::new("FEV1")
                .with_line(with_ssc)
                .with_line(other_ssc),
        )
        .invoice(InvoiceBuilder::new("FEV2").line("5"))
        .excluded("FEV3", "SIN_NT")
        .build();
    ctx.dataset().replace(&ds, false).expect("replace");
}

fn keys(ctx: &AppContext) -> Vec<String> {
    ctx.artifacts()
        .list(LOAD)
        .expect("list")
        .into_iter()
        .map(|a| a.rel_path)
        .collect()
}

#[test]
fn without_invoices_every_prepared_invoice_runs_with_the_ssc_of_its_first_line() {
    let db = TestDb::new();
    let s = servers();
    let ctx = context(&db, &s);
    seed_dataset(&db, &ctx);

    let report = thefactory::run_phase(&ctx, &PhaseRequest::new(LOAD)).expect("run_phase");
    let done: HashMap<_, _> = report
        .outcomes
        .iter()
        .map(|o| (o.invoice.as_str(), o.outcome.state))
        .collect();
    assert_eq!(done.len(), 2, "the excluded invoice is not downloaded");
    assert!(done.values().all(|s| *s == PhaseState::Done));
    let keys = keys(&ctx);
    assert!(keys.contains(&"CPSCAN/FEV1.cpscan.json".to_owned()));
    assert!(keys.contains(&"CPSCAN/FEV1.support.jpg".to_owned()));
    assert!(
        !keys.iter().any(|k| k.contains("FEV2.cpscan")),
        "FEV2 has no SSC"
    );
    assert!(!keys.iter().any(|k| k.contains("FEV3")));
    let paths: Vec<String> = s.api.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(
        paths,
        ["/documentos/100"],
        "SSC normalized from the first line"
    );
}

#[test]
fn explicit_invoices_still_take_their_ssc_from_the_dataset() {
    let db = TestDb::new();
    let s = servers();
    let ctx = context(&db, &s);
    seed_dataset(&db, &ctx);

    let mut request = PhaseRequest::new(LOAD);
    request.invoices = Some(vec!["FEV1".into(), "FEV9".into()]);
    let report = thefactory::run_phase(&ctx, &request).expect("run_phase");
    assert_eq!(report.outcomes.len(), 2);
    assert_eq!(
        s.api.requests().len(),
        1,
        "FEV9 is not in the dataset: no CPScan part"
    );
    let rows = ctx.proceso().load(Phase::TheFactory, None).expect("states");
    assert_eq!(rows.len(), 2);
}

#[test]
fn a_missing_dataset_is_an_error_without_invoices_and_a_note_with_them() {
    let db = TestDb::new();
    let s = servers();
    let ctx = context(&db, &s);

    let err = thefactory::run_phase(&ctx, &PhaseRequest::new(LOAD)).expect_err("no dataset");
    assert!(err.to_string().contains("--invoices"), "{err}");
    assert!(s.tf.requests().is_empty());

    let mut request = PhaseRequest::new(LOAD);
    request.invoices = Some(vec!["FEV1".into()]);
    let report = thefactory::run_phase(&ctx, &request).expect("only the CPScan part is lost");
    assert_eq!(report.counters().done, 1);
    assert!(s.api.requests().is_empty());
}
