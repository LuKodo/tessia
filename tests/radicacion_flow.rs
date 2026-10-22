//! `radicacion` end to end: artifacts in the in-memory bucket, a scripted Mutual
//! Ser server, real Postgres state and the dataset store for the value check.

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rust_decimal::Decimal;
use serde_json::{Value, json};
use tessia::config::Settings;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::PhaseState;
use tessia::domain::dataset::test_support::{DatasetBuilder, InvoiceBuilder};
use tessia::domain::radicacion::{RadStatus, Step};
use tessia::phases::common::PhaseRequest;
use tessia::phases::radicacion::engine::{Config, Wait};
use tessia::phases::radicacion::portal::PortalStore;
use tessia::phases::radicacion::{self, RunOptions, TIMINGS_ARTIFACT};
use tessia::stores::dataset::DatasetStore;
use tessia::stores::dataset::test_support::seed_cargue;
use tessia::testing::{MockResponse, MockServer, RecordedRequest};

const CARGUE: &str = "Convenio_21102026_21102026";
const NIT: &str = "900111222";

/// Mutual Ser as a healthy portal: remembers what was registered under each
/// upload id and confirms everything.
#[derive(Default)]
struct Portal {
    names: HashMap<String, Vec<String>>,
    finished: std::collections::HashSet<String>,
    rips: Vec<String>,
    crc: Vec<String>,
    xml_status_type: Option<&'static str>,
}

fn portal_handler(state: Arc<Mutex<Portal>>) -> impl Fn(&RecordedRequest) -> MockResponse + Send {
    move |req| {
        let path = req.path.split('?').next().unwrap_or_default().to_owned();
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let mut portal = state.lock().expect("portal");
        match path.as_str() {
            "/login" => MockResponse::json(200, r#"{"access_token":"tok","expires_in":3600}"#),
            "/rips-api/upload" => {
                let id = body["id_cargue"].as_str().unwrap_or_default().to_owned();
                let names: Vec<String> = body["nombres"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|n| n.as_str().map(str::to_owned))
                    .collect();
                if names.iter().any(|n| n.ends_with(".json")) {
                    portal.rips.extend(
                        names
                            .iter()
                            .filter_map(|n| n.strip_suffix(".json").map(str::to_owned)),
                    );
                }
                if names.iter().any(|n| n.starts_with("CRC_")) {
                    portal.crc.extend(names.iter().filter_map(|n| {
                        n.rsplit('_')
                            .next()
                            .and_then(|n| n.strip_suffix(".zip"))
                            .map(str::to_owned)
                    }));
                }
                portal.names.insert(id, names);
                MockResponse::json(200, "{}")
            }
            "/rips-api/findLoad" => {
                let id = body["id"].as_str().unwrap_or_default();
                if !portal.finished.contains(id) {
                    return MockResponse::json(200, r#"{"estado":"EN_PROCESO","archivos":[]}"#);
                }
                let archivos: Vec<Value> = portal.names[id]
                    .iter()
                    .map(|n| json!({"id": n, "nombre": n, "estado": "CARGADO", "mensajes": []}))
                    .collect();
                MockResponse::json(
                    200,
                    &json!({"estado": "TERMINADO", "archivos": archivos}).to_string(),
                )
            }
            "/rips-api/signedUrl/getUrlUploadFile" => {
                let host = req.header("host").unwrap_or_default().to_owned();
                let names = req
                    .path
                    .split("fileNames=")
                    .nth(1)
                    .unwrap_or_default()
                    .replace("%2C", ",");
                let map: serde_json::Map<String, Value> = names
                    .split(',')
                    .map(|n| {
                        (
                            n.to_owned(),
                            json!(format!("http://{host}/storage/{n}?sig=x")),
                        )
                    })
                    .collect();
                MockResponse::json(200, &Value::Object(map).to_string())
            }
            "/rips-api/upload-files" => {
                for entry in body.as_array().into_iter().flatten() {
                    portal
                        .finished
                        .insert(entry["id_cargue"].as_str().unwrap_or_default().to_owned());
                }
                MockResponse::json(200, "{}")
            }
            p if p.starts_with("/storage/") => MockResponse::text(200, ""),
            "/rips-api/filter-xml/" => {
                let invoice = body["documentNumber"].as_str().unwrap_or_default();
                let (status, kind) = match portal.xml_status_type {
                    Some(kind) => ("INACTIVE", kind),
                    None => ("ACTIVE", "2"),
                };
                MockResponse::json(200, &json!({"content": [{"billId": invoice, "status": status, "statusType": kind}]}).to_string())
            }
            "/rips-api/core/filtersAll" => {
                let invoice = body["filtros"]["numero_factura"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if !portal.rips.contains(&invoice) {
                    return MockResponse::json(200, r#"{"contenido":[]}"#);
                }
                let crc = portal.crc.contains(&invoice);
                let mut row = json!({
                    "numero_factura": invoice, "estado": "VALIDADO_CON_ADVERTENCIA", "servicios": "[1]", "isSupportDocument": crc,
                    "nombreArchivo": format!("{invoice}.json"), "idArchivo": format!("f-{invoice}"),
                    "fecha_cargue": "29-09-2026 17:01:26"
                });
                if crc {
                    row["documento_soporte"] = json!({"id": "s1", "estado": "ACTIVE"});
                }
                MockResponse::json(200, &json!({"contenido": [row]}).to_string())
            }
            p if p.starts_with("/rips-api/core/findDetailsByFile/") => MockResponse::json(
                200,
                r#"[{"codigo":"A1","tipo":"ADVERTENCIA","campo":"edad","detalle":"revisar"}]"#,
            ),
            _ => MockResponse::text(404, ""),
        }
    }
}

fn invoice_zip(total: &str) -> Vec<u8> {
    let xml = format!(
        "<AttachedDocument><cbc:Description><![CDATA[<Invoice><cac:LegalMonetaryTotal><cbc:PayableAmount currencyID=\"COP\">{total}</cbc:PayableAmount></cac:LegalMonetaryTotal></Invoice>]]></cbc:Description></AttachedDocument>"
    );
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut zw = zip::ZipWriter::new(&mut buf);
    zw.start_file("FEV.xml", zip::write::SimpleFileOptions::default())
        .expect("entry");
    zw.write_all(xml.as_bytes()).expect("bytes");
    zw.finish().expect("finish");
    buf.into_inner()
}

struct Fixture {
    db: TestDb,
    ctx: AppContext,
    server: MockServer,
    portal: Arc<Mutex<Portal>>,
}

fn fast_options() -> RunOptions {
    RunOptions {
        engine: Config {
            batch_wait: Duration::from_millis(60),
            first_poll_delay: Duration::from_millis(10),
            poll_interval: Duration::from_millis(20),
            poll_max_interval: Duration::from_millis(100),
            quick_retry_delay: Duration::from_millis(20),
            retry_interval: Duration::from_millis(100),
            reload_wait: Wait::Fixed(Duration::from_millis(10)),
            deadline: Duration::from_secs(30),
            ..Config::default()
        },
        portal_follow: false,
        portal_schedule: Vec::new(),
    }
}

fn fixture(invoices: &[(&str, &str, &str)]) -> Fixture {
    let db = TestDb::new();
    let portal = Arc::new(Mutex::new(Portal::default()));
    let server = MockServer::start(portal_handler(portal.clone()));
    let url = server.url().to_owned();
    let settings = Settings::from_pairs([
        ("TESSIA_ZONASER_LOGIN_URL", format!("{url}/login")),
        (
            "TESSIA_ZONASER_BENAVENTE_BASE_URL",
            format!("{url}/rips-api"),
        ),
        ("TESSIA_ZONASER_USERNAME", "user".to_owned()),
        ("TESSIA_ZONASER_PASSWORD", "secret".to_owned()),
        ("TESSIA_ZONASER_ORGANIZATION_NIT", NIT.to_owned()),
    ]);
    let ctx = AppContext::for_tests(&db).with_settings(settings);
    let cargue = seed_cargue(&db.pool(), CARGUE, 'a');
    let mut dataset = DatasetBuilder::new(cargue).load(CARGUE);
    let artifacts = ctx.artifacts();
    for (invoice, xml_total, agreement) in invoices {
        dataset = dataset.invoice(InvoiceBuilder::new(invoice).line(agreement));
        artifacts
            .publish_bytes(
                CARGUE,
                &format!("XML/{invoice}_XML_PDF.zip"),
                &invoice_zip(xml_total),
            )
            .expect("zip");
        artifacts
            .publish_bytes(
                CARGUE,
                &format!("{invoice}.txt"),
                br#"{"CodigoUnicoValidacion":"abc"}"#,
            )
            .expect("txt");
        artifacts
            .publish_bytes(CARGUE, &format!("{invoice}.json"), b"{\"servicios\":[]}")
            .expect("json");
        artifacts
            .publish_bytes(CARGUE, &format!("CRC/CRC_{NIT}_{invoice}.zip"), b"crc")
            .expect("crc");
    }
    DatasetStore::new(db.pool())
        .replace(&dataset.build(), false)
        .expect("dataset");
    Fixture {
        db,
        ctx,
        server,
        portal,
    }
}

impl Fixture {
    fn calls(&self, suffix: &str) -> usize {
        self.server
            .requests()
            .iter()
            .filter(|r| {
                r.path
                    .split('?')
                    .next()
                    .unwrap_or_default()
                    .ends_with(suffix)
            })
            .count()
    }
}

#[test]
fn run_phase_files_every_invoice_of_the_cargue_through_the_four_steps() {
    let f = fixture(&[("FEV1", "1,000.00", "1000"), ("FEV2", "250.50", "250.50")]);
    let report = radicacion::run_phase_with(&f.ctx, &PhaseRequest::new(CARGUE), &fast_options())
        .expect("run");
    let states: Vec<_> = report
        .outcomes
        .iter()
        .map(|o| (o.invoice.as_str(), o.outcome.state))
        .collect();
    assert_eq!(
        states,
        [("FEV1", PhaseState::Done), ("FEV2", PhaseState::Done)]
    );
    assert!(report.skipped.is_empty());

    let stored = f
        .ctx
        .radicacion()
        .load(&["FEV1".into(), "FEV2".into()])
        .expect("states");
    for invoice in ["FEV1", "FEV2"] {
        let st = &stored[invoice];
        assert_eq!(
            (st.step, st.status),
            (Step::Crc, RadStatus::Confirmed),
            "{invoice}"
        );
        assert_eq!(st.folder, CARGUE);
    }
    assert_eq!(
        f.calls("/upload"),
        4,
        "one micro-batch upload per step for both invoices"
    );
    assert_eq!(f.calls("/filter-xml/"), 2);
    let xml = f
        .ctx
        .valor_xml()
        .stored_xml_total("FEV1")
        .expect("stored")
        .expect("row");
    assert_eq!(xml.0, Decimal::new(100000, 2));

    let timings = f
        .ctx
        .artifacts()
        .fetch_bytes(CARGUE, TIMINGS_ARTIFACT)
        .expect("timings artifact");
    let json: Value = serde_json::from_slice(&timings).expect("json");
    assert_eq!(json["version"], 1);
    assert_eq!(json["steps"].as_array().map(Vec::len), Some(4));

    // Filed artifacts are marked, so a second run without invoices finds nothing to file.
    assert!(
        radicacion::run_cargue(&f.ctx, CARGUE, None, &fast_options())
            .expect("rerun")
            .is_none()
    );
    // ...and asking for them explicitly reports them done without any call.
    let before = f.server.requests().len();
    let again = radicacion::run_phase_with(
        &f.ctx,
        &PhaseRequest {
            invoices: Some(vec!["FEV1".into()]),
            ..PhaseRequest::new(CARGUE)
        },
        &fast_options(),
    )
    .expect("again");
    assert_eq!(again.outcomes.len(), 1);
    assert!(
        f.server.requests().len() <= before + 1,
        "only the login at most"
    );
    drop(f.db);
}

#[test]
fn a_value_mismatch_blocks_step_1_before_any_upload() {
    let f = fixture(&[("FEV1", "1000.00", "900")]);
    let report = radicacion::run_phase_with(&f.ctx, &PhaseRequest::new(CARGUE), &fast_options())
        .expect("run");
    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(report.outcomes[0].outcome.state, PhaseState::Failed);
    assert_eq!(report.outcomes[0].outcome.motivo, "VALOR_FACTURA_DIFERENTE");
    assert_eq!(f.calls("/upload"), 0);
    let st = f
        .ctx
        .radicacion()
        .load(&["FEV1".into()])
        .expect("states")
        .remove("FEV1")
        .expect("row");
    assert_eq!(st.status, RadStatus::Blocked);
    assert!(
        st.last_detail
            .starts_with("codigo=VALOR_FACTURA_DIFERENTE mensaje="),
        "{}",
        st.last_detail
    );
}

#[test]
fn another_machine_holding_the_cargue_lease_stops_the_run() {
    let f = fixture(&[("FEV1", "1000.00", "1000")]);
    let other = f.ctx.clone().with_owner("otra-maquina:1:abc");
    let held = other
        .leases()
        .guard(
            &format!("radicacion:{CARGUE}"),
            &other.owner,
            Duration::from_secs(60),
        )
        .expect("lease")
        .expect("free");
    let err = radicacion::run_phase_with(&f.ctx, &PhaseRequest::new(CARGUE), &fast_options())
        .expect_err("must refuse");
    assert!(
        err.to_string()
            .contains("ya lo está radicando otra máquina (otra-maquina:1:abc)"),
        "{err}"
    );
    assert_eq!(f.calls("/upload"), 0);
    drop(held);
    // Released: the run now proceeds (and frees its own lease at the end).
    radicacion::run_phase_with(&f.ctx, &PhaseRequest::new(CARGUE), &fast_options()).expect("run");
    assert_eq!(
        f.ctx
            .leases()
            .holder(&format!("radicacion:{CARGUE}"))
            .expect("holder"),
        None
    );
}

#[test]
fn missing_credentials_are_reported_by_key() {
    let f = fixture(&[("FEV1", "1000.00", "1000")]);
    let ctx = f.ctx.clone().with_settings(Settings::from_pairs([(
        "TESSIA_ZONASER_ORGANIZATION_NIT",
        NIT.to_owned(),
    )]));
    let err = radicacion::run_phase(&ctx, &PhaseRequest::new(CARGUE)).expect_err("must fail");
    let text = format!("{err:#}");
    assert!(
        text.contains("zonaser.login_url") && text.contains("zonaser.password"),
        "{text}"
    );
    assert!(!text.contains("secret"));
}

#[test]
fn recheck_step2_records_a_rejected_cuv_as_an_adjustment_note() {
    let f = fixture(&[("FEV1", "1000.00", "1000")]);
    f.portal.lock().expect("portal").xml_status_type = Some("3");
    let args = radicacion::Args {
        command: radicacion::cli::Command::RecheckStep2(radicacion::cli::RecheckStep2Args {
            invoices: vec!["fev-1".into()],
        }),
    };
    radicacion::run(&f.ctx, args).expect("recheck");
    let st = f
        .ctx
        .radicacion()
        .load(&["FEV1".into()])
        .expect("states")
        .remove("FEV1")
        .expect("row");
    assert_eq!((st.step, st.status), (Step::Cuv, RadStatus::AdjustmentNote));
    assert!(
        st.last_detail
            .starts_with("paso=2 filter-xml reverificacion factura=FEV1 encontrado=true"),
        "{}",
        st.last_detail
    );
    assert!(
        st.last_detail
            .ends_with("statusType=3 id= errores= confirmado=false"),
        "{}",
        st.last_detail
    );

    let empty = radicacion::Args {
        command: radicacion::cli::Command::RecheckStep2(
            radicacion::cli::RecheckStep2Args::default(),
        ),
    };
    assert!(radicacion::run(&f.ctx, empty).is_err());
}

#[test]
fn portal_follow_stores_the_portal_state_of_each_uploaded_rips_json() {
    let f = fixture(&[("FEV1", "1000.00", "1000")]);
    let mut opts = fast_options();
    opts.portal_follow = true;
    opts.portal_schedule = vec![Duration::from_millis(20); 3];
    let report =
        radicacion::run_phase_with(&f.ctx, &PhaseRequest::new(CARGUE), &opts).expect("run");
    assert_eq!(report.outcomes[0].outcome.state, PhaseState::Done);
    let stored = PortalStore::new(f.db.pool())
        .get("FEV1")
        .expect("get")
        .expect("followed");
    assert_eq!(stored.record.status, "VALIDADO_CON_ADVERTENCIA");
    assert!(stored.is_final);
    assert_eq!(
        (stored.warnings, stored.errors, stored.findings.len()),
        (1, 0, 1)
    );
    assert_eq!(f.calls("/findDetailsByFile/f-FEV1"), 1);

    // Off by default: the other cargue-less run makes no portal calls.
    let f = fixture(&[("FEV1", "1000.00", "1000")]);
    radicacion::run_phase_with(&f.ctx, &PhaseRequest::new(CARGUE), &fast_options()).expect("run");
    assert_eq!(f.calls("/findDetailsByFile/f-FEV1"), 0);
    assert!(
        PortalStore::new(f.db.pool())
            .get("FEV1")
            .expect("get")
            .is_none()
    );
}
