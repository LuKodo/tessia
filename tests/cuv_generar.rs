//! `cuv generar` end to end against a local SISPRO, an in-memory bucket and a
//! throwaway Postgres: payload and XML goldens, gates, skip codes, every
//! response class, the login alert, claims and leases between machines.

#[path = "cuv_support.rs"]
mod support;

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Barrier};

use serde_json::Value;
use support::*;
use tessia::db::test_support::TestDb;
use tessia::domain::{Outcome, Phase, PhaseState};
use tessia::fevxml;
use tessia::phases::common::{PhaseReport, PhaseRequest};
use tessia::phases::cuv::{
    self, MissingRipsJsonError, TokenTally, check_alert, generar, lease_name, run_phase,
    run_phase_tallied,
};
use tessia::stores::parametros::Parametros;

fn request(invoices: Option<&[&str]>) -> PhaseRequest {
    PhaseRequest {
        cargue_id: CARGUE.to_owned(),
        invoices: invoices.map(|l| l.iter().map(|s| (*s).to_owned()).collect()),
        workers: Some(4),
    }
}

fn xml_case(name: &str) -> (String, String, Vec<u8>, Vec<u8>) {
    let manifest: Value = serde_json::from_slice(&read("xml.json")).expect("manifest");
    let case = manifest["xml"]
        .as_array()
        .and_then(|cases| cases.iter().find(|c| c["name"] == name))
        .unwrap_or_else(|| panic!("case {name}"));
    let regimen = case["regimen"].as_str().unwrap_or_default().to_owned();
    let contrato = case["contrato"].as_str().unwrap_or_default().to_owned();
    let input = read(&format!("xml/{name}.input.xml"));
    let ministerio = case["ministerio"]
        .as_str()
        .map(|_| read(&format!("xml/{name}.ministerio.xml")))
        .unwrap_or_default();
    (regimen, contrato, input, ministerio)
}

fn zip_of(invoice: &str, xml: &[u8]) -> Vec<u8> {
    fevxml::build_xml_pdf_zip(
        &format!("{invoice}.pdf"),
        b"%PDF-1.4 synthetic",
        &format!("{invoice}.xml"),
        xml,
        None,
    )
    .expect("zip")
}

fn zip_names(bytes: &[u8]) -> Vec<String> {
    let archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    archive
        .file_names()
        .filter_map(|n| n.ok().map(|c| c.into_owned()))
        .collect()
}

fn outcome_of(report: &PhaseReport, invoice: &str) -> Outcome {
    report
        .outcomes
        .iter()
        .find(|o| o.invoice == invoice)
        .unwrap_or_else(|| panic!("no outcome for {invoice}: {report:?}"))
        .outcome
        .clone()
}

fn stored(ctx: &tessia::context::AppContext, invoice: &str) -> (PhaseState, String) {
    let row = state(ctx, Phase::Cuv, invoice).unwrap_or_else(|| panic!("no state for {invoice}"));
    (row.state, row.motivo)
}

/// Seeds an invoice with its RIPS JSON and, optionally, its XML.
fn seed(ctx: &tessia::context::AppContext, invoice: &str, regimen: &str, xml: Option<&[u8]>) {
    publish(
        ctx,
        &format!("{invoice}.json"),
        &rips_json(invoice, regimen),
    );
    if let Some(xml) = xml {
        publish(ctx, &format!("{invoice}.xml"), xml);
    }
}

#[test]
fn sends_the_java_adjusted_xml_and_writes_the_swept_txt() {
    let db = TestDb::new();
    let (regimen, contrato, input, golden) = xml_case("contributivo");
    let inv = "FVS1000001";
    let fake = FakeSispro::start(answers(&[(
        inv,
        200,
        String::from_utf8(read("respuestas/exito.txt")).expect("exito"),
    )]));
    let ctx = context(&db, fake.url(), &contrato);
    seed(&ctx, inv, &regimen, Some(&input));
    publish(
        &ctx,
        &format!("XML/{inv}_XML_PDF.zip"),
        &zip_of(inv, &input),
    );

    let report = run_phase(&ctx, &request(None)).expect("run");

    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(outcome_of(&report, inv), Outcome::done());
    assert_eq!(stored(&ctx, inv), (PhaseState::Done, String::new()));
    // The TXT is the Gson response after the Java final sweep.
    assert_eq!(
        fetch(&ctx, &format!("{inv}.txt")),
        read("respuestas/exito.barrido.txt")
    );
    // The payload carries the golden XML (base64) and the RIPS bytes as stored.
    let sent = fake.cargas();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].xml, golden, "xmlFevFile is the prepared XML");
    assert_eq!(
        sent[0].rips,
        rips_json(inv, &regimen),
        "rips is the JSON file, byte for byte"
    );
    // The adjusted XML is written back and replaces the ZIP entry (appended last).
    assert_eq!(fetch(&ctx, &format!("{inv}.xml")), golden);
    let zip = fetch(&ctx, &format!("XML/{inv}_XML_PDF.zip"));
    assert_eq!(
        zip_names(&zip),
        [format!("{inv}.pdf"), format!("{inv}.xml")]
    );
    assert_eq!(
        fevxml::read_entry(&zip, &format!("{inv}.xml"))
            .expect("zip")
            .expect("entry"),
        golden
    );
    assert_eq!(fake.count(LOGIN), 1);
    assert_eq!(fake.count(CARGAR), 1);
}

#[test]
fn a_second_run_skips_the_invoice_that_already_has_its_cuv() {
    let db = TestDb::new();
    let (regimen, contrato, input, _) = xml_case("contributivo");
    let inv = "FVS1000001";
    let fake = FakeSispro::start(answers(&[(inv, 200, success_body(inv))]));
    let ctx = context(&db, fake.url(), &contrato);
    seed(&ctx, inv, &regimen, Some(&input));
    run_phase(&ctx, &request(None)).expect("first");
    assert_eq!(fake.count(CARGAR), 1);
    let second = run_phase(&ctx, &request(Some(&[inv]))).expect("second");
    assert_eq!(fake.count(CARGAR), 1, "YA_TIENE_CUV: nothing is sent again");
    assert_eq!(outcome_of(&second, inv), Outcome::done());
    assert_eq!(state(&ctx, Phase::Cuv, inv).expect("row").intentos, 2);
}

#[test]
fn xml_only_in_the_zip_is_restored_and_the_period_error_keeps_the_edits() {
    let db = TestDb::new();
    // Restored from the ZIP, régimen subsidiado.
    let (regimen, contrato, input, golden) = xml_case("subsidiado");
    let inv = "FVS1000002";
    let fake = FakeSispro::start(answers(&[(inv, 200, success_body(inv))]));
    let ctx = context(&db, fake.url(), &contrato);
    seed(&ctx, inv, &regimen, None);
    publish(
        &ctx,
        &format!("XML/{inv}_XML_PDF.zip"),
        &zip_of(inv, &input),
    );
    run_phase(&ctx, &request(None)).expect("run");
    assert_eq!(fake.cargas()[0].xml, golden);
    assert_eq!(
        fetch(&ctx, &format!("{inv}.xml")),
        golden,
        "restored and adjusted"
    );

    // Period error: the earlier edits are kept, the ZIP is not refreshed and the
    // XML is sent anyway with an `aviso`.
    let db = TestDb::new();
    let (regimen, contrato, input, golden) = xml_case("periodo_parcial");
    let inv = "FVS1000003";
    let fake = FakeSispro::start(answers(&[(inv, 200, success_body(inv))]));
    let ctx = context(&db, fake.url(), &contrato);
    seed(&ctx, inv, &regimen, Some(&input));
    let original_zip = zip_of(inv, &input);
    publish(&ctx, &format!("XML/{inv}_XML_PDF.zip"), &original_zip);
    let tally = TokenTally::new();
    let execution = generar::execute(&ctx, &request(None), &tally).expect("run");
    assert_eq!(fake.cargas()[0].xml, golden, "sent with the earlier edits");
    assert_eq!(fetch(&ctx, &format!("{inv}.xml")), golden);
    assert_eq!(fetch(&ctx, &format!("XML/{inv}_XML_PDF.zip")), original_zip);
    assert!(
        execution
            .lines
            .iter()
            .any(|l| l
                .contains("[aviso: fevxml: no se pudo garantizar la fecha inicial del periodo")),
        "{:?}",
        execution.lines
    );
}

#[test]
fn skip_codes_are_recorded_without_contacting_sispro() {
    let db = TestDb::new();
    // No `minsalud.base_url` at all: the client is only built when something is sent.
    let (regimen, contrato, input, _) = xml_case("contributivo");
    let (_, _, no_contract, _) = xml_case("sin_contrato");
    let ctx = context(&db, "", &contrato);
    // YA_TIENE_CUV (valid CUV in the TXT).
    seed(&ctx, "FVS1", &regimen, Some(&input));
    publish(&ctx, "FVS1.txt", success_body("FVS1").as_bytes());
    // XML_FALTA: nothing, and a ZIP without the entry.
    seed(&ctx, "FVS2", &regimen, None);
    seed(&ctx, "FVS3", &regimen, None);
    let other =
        fevxml::build_xml_pdf_zip("FVS3.pdf", b"%PDF-1.4", "OTRO.xml", b"<a/>", None).expect("zip");
    publish(&ctx, "XML/FVS3_XML_PDF.zip", &other);
    // XML_SIN_CONTRATO and RIPS_INVALIDO.
    seed(&ctx, "FVS4", &regimen, Some(&no_contract));
    publish(&ctx, "FVS5.json", b"[1,2]");
    publish(&ctx, "FVS5.xml", &input);

    let report = run_phase(&ctx, &request(None)).expect("run");

    assert_eq!(outcome_of(&report, "FVS1"), Outcome::done());
    assert_eq!(
        outcome_of(&report, "FVS2"),
        Outcome::failed("XML_FALTA (ejecute `tessia thefactory descargar`)")
    );
    assert_eq!(
        outcome_of(&report, "FVS3"),
        Outcome::failed("XML_FALTA (no esta en FVS3_XML_PDF.zip)")
    );
    let sin_contrato = outcome_of(&report, "FVS4");
    assert_eq!(sin_contrato.state, PhaseState::RevisionManual);
    assert!(
        sin_contrato
            .motivo
            .starts_with("XML_SIN_CONTRATO (fevxml: XML sin NUMERO_CONTRATO"),
        "{}",
        sin_contrato.motivo
    );
    assert_eq!(
        outcome_of(&report, "FVS5"),
        Outcome::manual_review("RIPS_INVALIDO (cuv: el JSON RIPS no es un objeto JSON valido)")
    );
    for invoice in ["FVS1", "FVS2", "FVS3", "FVS4", "FVS5"] {
        let row = state(&ctx, Phase::Cuv, invoice).expect("stored");
        assert_eq!(row.state, outcome_of(&report, invoice).state, "{invoice}");
    }
    assert!(!published(&ctx, "FVS4.txt"));
}

#[test]
fn gates_apply_only_without_explicit_invoices() {
    let db = TestDb::new();
    let (regimen, contrato, input, _) = xml_case("contributivo");
    let names = ["FVS1", "FVS2", "FVS3", "FVS4"];
    let fake = FakeSispro::start(
        names
            .iter()
            .map(|n| ((*n).to_owned(), (200, success_body(n))))
            .collect(),
    );
    let ctx = context(&db, fake.url(), &contrato);
    for n in names {
        seed(&ctx, n, &regimen, Some(&input));
    }
    save_state(&ctx, Phase::Rips, "FVS1", Outcome::failed("RIPS roto"));
    save_state(
        &ctx,
        Phase::Cuv,
        "FVS2",
        Outcome::manual_review("rechazada"),
    );
    save_state(&ctx, Phase::Rips, "FVS4", Outcome::done());

    let tally = TokenTally::new();
    let execution = generar::execute(&ctx, &request(None), &tally).expect("run");
    assert_eq!(
        fake.sent_invoices(),
        ["FVS3", "FVS4"],
        "FVS3 has no rips state: still sent"
    );
    assert_eq!(
        execution.report.skipped,
        [
            ("FVS1".to_owned(), "RIPS_NO_OK".to_owned()),
            ("FVS2".to_owned(), "REVISION_MANUAL".to_owned())
        ]
    );
    assert!(execution.lines.contains(
        &"  skip FVS1 RIPS_NO_OK: rips FAILED: RIPS roto (reprocess with --invoices)".to_owned()
    ));
    assert!(execution.lines.contains(&"  skip FVS2 REVISION_MANUAL: cuv REVISION_MANUAL: rechazada (reprocess with --invoices)".to_owned()));
    // The gated invoices keep their stored state.
    assert_eq!(
        stored(&ctx, "FVS2"),
        (PhaseState::RevisionManual, "rechazada".to_owned())
    );
    assert!(state(&ctx, Phase::Cuv, "FVS1").is_none());

    // Asked for explicitly, both are sent again whatever their state.
    run_phase(&ctx, &request(Some(&["FVS1", "FVS2"]))).expect("explicit");
    assert_eq!(fake.sent_invoices(), ["FVS1", "FVS2", "FVS3", "FVS4"]);
    assert_eq!(stored(&ctx, "FVS2").0, PhaseState::Done);
}

#[test]
fn every_sispro_response_class_ends_in_the_right_file_and_state() {
    let db = TestDb::new();
    let (regimen, contrato, input, _) = xml_case("contributivo");
    let resp =
        |name: &str| String::from_utf8(read(&format!("respuestas/{name}.txt"))).expect("utf8");
    // The rejection golden is for FVS1000003: retarget it to the invoice at hand.
    let rechazo_for = |invoice: &str| resp("rechazo").replace("FVS1000003", invoice);
    let other = r#"{"NumFactura":"OTRA9","ResultadosValidacion":[]}"#;
    let error_400 = r#"{"NumFactura":"FVS105","ResultadosValidacion":[{"Clase":"NOTIFICACION","Codigo":"X","PathFuente":"p"}]}"#;
    let cases = [
        ("FVS101", 200, success_body("FVS101")),
        ("FVS102", 200, rechazo_for("FVS102")),
        ("FVS103", 201, resp("rvg18")),
        ("FVS104", 400, rechazo_for("FVS104")),
        ("FVS105", 400, error_400.to_owned()),
        ("FVS106", 400, resp("rvg18")),
        ("FVS107", 200, other.to_owned()),
        ("FVS108", 500, r#"{"ResultadosValidacion":[]}"#.to_owned()),
        ("FVS109", 404, r#"{"ResultadosValidacion":[]}"#.to_owned()),
        ("FVS110", 200, "<html>gateway</html>".to_owned()),
        (
            "FVS111",
            200,
            r#"{"ResultadosValidacion":[{"Codigo":"X"}]}"#.to_owned(),
        ),
        ("FVS112", 400, r#"{"NumFactura":"FVS112"}"#.to_owned()),
    ];
    let fake = FakeSispro::start(
        cases
            .iter()
            .map(|(i, s, b)| ((*i).to_owned(), (*s, b.clone())))
            .collect(),
    );
    let ctx = context(&db, fake.url(), &contrato);
    for (invoice, _, _) in &cases {
        seed(&ctx, invoice, &regimen, Some(&input));
    }

    let report = run_phase(&ctx, &request(None)).expect("run");

    let rechazo =
        tessia::phases::cuv::model::Response::parse(read("respuestas/rechazo.txt").as_slice())
            .expect("rechazo");
    let why = tessia::phases::cuv::model::rejection_summary(&rechazo);
    assert!(why.starts_with("\"RECHAZADO\" - \""), "{why}");
    let rvg18 =
        tessia::phases::cuv::model::Response::parse(read("respuestas/rvg18.txt").as_slice())
            .expect("rvg18");
    let why18 = tessia::phases::cuv::model::rejection_summary(&rvg18);
    let expect = |invoice: &str, outcome: Outcome, files: &[&str]| {
        assert_eq!(outcome_of(&report, invoice), outcome, "{invoice}");
        assert_eq!(
            stored(&ctx, invoice),
            (outcome.state, outcome.motivo.clone()),
            "{invoice}"
        );
        for suffix in ["", "_ERROR", "_RESPUESTA_NO_CORRESPONDE"] {
            let name = format!("{invoice}{suffix}.txt");
            assert_eq!(
                published(&ctx, &name),
                files.contains(&name.as_str()),
                "{name}"
            );
        }
    };
    expect("FVS101", Outcome::done(), &["FVS101.txt"]);
    // Rejected with RECHAZADO results: the reason is what SISPRO rejected it for.
    expect(
        "FVS102",
        Outcome::manual_review(why.clone()),
        &["FVS102.txt"],
    );
    let barrido = |invoice: &str| {
        String::from_utf8(read("respuestas/rechazo.barrido.txt"))
            .expect("utf8")
            .replace("FVS1000003", invoice)
            .into_bytes()
    };
    assert_eq!(fetch(&ctx, "FVS102.txt"), barrido("FVS102"));
    expect("FVS104", Outcome::manual_review(why), &["FVS104_ERROR.txt"]);
    assert_eq!(fetch(&ctx, "FVS104_ERROR.txt"), barrido("FVS104"));
    // RVG18: the sweep puts the existing CUV in the TXT (ProcesoId from ConsultarCUV).
    expect("FVS103", Outcome::done(), &["FVS103.txt"]);
    assert_eq!(
        fetch(&ctx, "FVS103.txt"),
        read("respuestas/rvg18.barrido.txt")
    );
    expect(
        "FVS106",
        Outcome::manual_review(format!("{why18} (RVG18: CUV existente)")),
        &["FVS106_ERROR.txt"],
    );
    assert_eq!(
        fetch(&ctx, "FVS106_ERROR.txt"),
        read("respuestas/rvg18.barrido.txt")
    );
    assert_eq!(fake.count(CONSULTA), 2);
    // A 400 without RECHAZADO keeps the status line.
    expect(
        "FVS105",
        Outcome::manual_review("HTTP 400 Error 400 -> FVS105_ERROR.txt"),
        &["FVS105_ERROR.txt"],
    );
    // A response for another invoice becomes the diagnostic file.
    expect(
        "FVS107",
        Outcome::manual_review(
            "HTTP 200 Respuesta no corresponde: OTRA9 -> FVS107_RESPUESTA_NO_CORRESPONDE.txt",
        ),
        &["FVS107_RESPUESTA_NO_CORRESPONDE.txt"],
    );
    let diagnostic =
        String::from_utf8(fetch(&ctx, "FVS107_RESPUESTA_NO_CORRESPONDE.txt")).expect("utf8");
    assert!(diagnostic.starts_with("Respuesta no corresponde a la factura local.\nFactura local: FVS107\nFactura respuesta Ministerio: OTRA9\nRespuesta:\n{"));
    // Server errors are retried later; other statuses and Java's NPE cases write nothing.
    expect("FVS108", Outcome::failed("HTTP 500 Error general"), &[]);
    expect(
        "FVS109",
        Outcome::manual_review("HTTP 404 Error general"),
        &[],
    );
    expect(
        "FVS110",
        Outcome::failed("HTTP 200 Error 500 al consumir el servicio"),
        &[],
    );
    expect(
        "FVS111",
        Outcome::failed("HTTP 200 Error 500 al consumir el servicio"),
        &[],
    );
    expect(
        "FVS112",
        Outcome::failed("HTTP 400 Error 500 al consumir el servicio"),
        &[],
    );
    // CargarFevRips is sent once per invoice, whatever the answer.
    assert_eq!(fake.count(CARGAR), cases.len());
}

#[test]
fn login_failures_fail_the_run_after_saving_and_the_threshold_is_configurable() {
    let db = TestDb::new();
    let (regimen, contrato, input, _) = xml_case("contributivo");
    let names = ["FVS1", "FVS2", "FVS3", "FVS4"];
    let fake = FakeSispro::start_with(HashMap::new(), false);
    let ctx = context(&db, fake.url(), &contrato);
    for n in names {
        seed(&ctx, n, &regimen, Some(&input));
    }

    let err = run_phase(&ctx, &request(None)).expect_err("alert");
    assert!(
        err.to_string().starts_with(
            "SISPRO: 4 de 4 facturas enviadas fallaron al iniciar sesion sin token (100%)"
        ),
        "{err}"
    );
    for n in names {
        let (st, motivo) = stored(&ctx, n);
        assert_eq!(
            st,
            PhaseState::Failed,
            "{n}: invoice states are unchanged by the alert"
        );
        assert!(
            motivo.starts_with("Error: Token vacío (sispro: token vacio: "),
            "{motivo}"
        );
    }
    assert_eq!(fake.count(CARGAR), 0, "nothing is sent without a token");

    // `proceso run` aggregates: the phase does not judge, the caller does.
    let tally = TokenTally::new();
    run_phase_tallied(&ctx, &request(None), &tally).expect("tallied run does not alert");
    assert_eq!(tally.snapshot().submitted, 4);
    assert_eq!(tally.snapshot().token_failed, 4);
    assert!(check_alert(&ctx, &tally).is_err());

    // Stored overrides: a higher minimum sample, or 0 % to disable.
    let store = ctx.parametros();
    let mut params = store.get_or_default(100_000.into()).expect("defaults");
    params.cuv_min_sample = 10;
    store.set(&params).expect("save");
    assert!(
        check_alert(&ctx, &tally).is_ok(),
        "below the stored minimum sample"
    );
    params.cuv_min_sample = 4;
    params.cuv_token_failure_percent = 0;
    store.set(&params).expect("save");
    assert!(check_alert(&ctx, &tally).is_ok(), "0 % disables the alert");
    params = Parametros {
        cuv_token_failure_percent: 99,
        ..params
    };
    store.set(&params).expect("save");
    assert!(check_alert(&ctx, &tally).is_err(), "100 % is above 99 %");
    assert!(
        run_phase(&ctx, &request(Some(&["FVS1"]))).is_ok(),
        "one invoice is below the minimum sample"
    );
}

#[test]
fn another_machine_owning_an_invoice_is_left_alone() {
    let db = TestDb::new();
    let (regimen, contrato, input, _) = xml_case("contributivo");
    let fake = FakeSispro::start(
        ["FVS1", "FVS2", "FVS3"]
            .iter()
            .map(|n| ((*n).to_owned(), (200, success_body(n))))
            .collect(),
    );
    let ctx = context(&db, fake.url(), &contrato);
    let other = ctx.clone().with_owner("otra-maquina:1:abc");
    for n in ["FVS1", "FVS2", "FVS3"] {
        seed(&ctx, n, &regimen, Some(&input));
    }
    // The other machine holds a fresh claim on FVS1 and a lease on FVS2.
    let claimed = other
        .proceso()
        .claim_available(
            Phase::Cuv,
            &[tessia::stores::proceso::ClaimItem::new("FVS1")],
            &other.claim_options(CARGUE),
        )
        .expect("claim");
    assert!(claimed.contains_key("FVS1"));
    assert!(
        other
            .leases()
            .acquire(
                &lease_name(CARGUE, "FVS2"),
                &other.owner,
                std::time::Duration::from_secs(600)
            )
            .expect("lease")
    );

    let report = run_phase(&ctx, &request(None)).expect("run");

    assert_eq!(fake.sent_invoices(), ["FVS3"]);
    let mut skipped: Vec<_> = report
        .skipped
        .iter()
        .map(|(i, r)| (i.as_str(), r.as_str()))
        .collect();
    skipped.sort();
    assert_eq!(
        skipped,
        [
            ("FVS1", "EN_PROCESO_OTRA_MAQUINA"),
            ("FVS2", "EN_PROCESO_OTRA_MAQUINA")
        ]
    );
    let row = state(&ctx, Phase::Cuv, "FVS1").expect("row");
    assert_eq!(row.state, PhaseState::EnProgreso);
    assert_eq!(row.claimed_by.as_deref(), Some("otra-maquina:1:abc"));
    assert!(
        state(&ctx, Phase::Cuv, "FVS2").is_none(),
        "the claim taken before the lease was given back"
    );

    // Once the lease is free the invoice is processed.
    other
        .leases()
        .release(&lease_name(CARGUE, "FVS2"), &other.owner)
        .expect("release");
    run_phase(&ctx, &request(Some(&["FVS2"]))).expect("second run");
    assert_eq!(fake.sent_invoices(), ["FVS2", "FVS3"]);
    assert_eq!(stored(&ctx, "FVS2").0, PhaseState::Done);
}

#[test]
fn two_machines_split_the_cargue_and_each_invoice_is_sent_once() {
    let db = TestDb::new();
    let (regimen, contrato, input, _) = xml_case("contributivo");
    let names: Vec<String> = (1..=8).map(|n| format!("FVS{n}")).collect();
    let fake = FakeSispro::start(
        names
            .iter()
            .map(|n| (n.clone(), (200, success_body(n))))
            .collect(),
    );
    let first = context(&db, fake.url(), &contrato);
    let second = first.clone().with_owner("maquina-2:2:def");
    for n in &names {
        seed(&first, n, &regimen, Some(&input));
    }
    let start = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [first.clone(), second]
        .into_iter()
        .map(|ctx| {
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                run_phase(&ctx, &request(None)).expect("run")
            })
        })
        .collect();
    let reports: Vec<PhaseReport> = handles
        .into_iter()
        .map(|h| h.join().expect("machine"))
        .collect();

    let mut sent = fake.sent_invoices();
    sent.dedup();
    assert_eq!(sent.len(), 8, "every invoice was sent");
    assert_eq!(fake.count(CARGAR), 8, "and none twice");
    let mut done: Vec<String> = reports
        .iter()
        .flat_map(|r| r.outcomes.iter().map(|o| o.invoice.clone()))
        .collect();
    done.sort();
    assert_eq!(done, names, "the two reports partition the cargue");
    for n in &names {
        assert_eq!(stored(&first, n).0, PhaseState::Done);
    }
}

#[test]
fn explicit_invoices_resend_a_rejected_txt_but_not_one_with_cuv() {
    let db = TestDb::new();
    let (regimen, contrato, input, _) = xml_case("contributivo");
    let fake = FakeSispro::start(answers(&[
        ("FVS1", 200, success_body("FVS1")),
        ("FVS2", 200, success_body("FVS2")),
    ]));
    let ctx = context(&db, fake.url(), &contrato);
    seed(&ctx, "FVS1", &regimen, Some(&input));
    seed(&ctx, "FVS2", &regimen, Some(&input));
    publish(
        &ctx,
        "FVS1.txt",
        br#"{"ResultadosValidacion":[{"Clase":"RECHAZADO"}]}"#,
    );
    publish(&ctx, "FVS1_ERROR.txt", b"{}");
    publish(&ctx, "FVS2.txt", success_body("FVS2").as_bytes());
    run_phase(&ctx, &request(Some(&["FVS1", "FVS2"]))).expect("run");
    assert_eq!(fake.sent_invoices(), ["FVS1"]);
    assert_eq!(stored(&ctx, "FVS1").0, PhaseState::Done);
    assert_eq!(stored(&ctx, "FVS2").0, PhaseState::Done);
    assert!(fetch(&ctx, "FVS1.txt").starts_with(b"{\n  \"ResultState\": true"));
}

#[test]
fn requested_invoices_without_rips_json_are_reported() {
    let db = TestDb::new();
    let (regimen, contrato, input, _) = xml_case("contributivo");
    let fake = FakeSispro::start(answers(&[("FVS1", 200, success_body("FVS1"))]));
    let ctx = context(&db, fake.url(), &contrato);
    seed(&ctx, "FVS1", &regimen, Some(&input));

    let err = run_phase(&ctx, &request(Some(&["NOPE9"]))).expect_err("single unknown");
    let typed = err
        .downcast_ref::<MissingRipsJsonError>()
        .expect("typed error");
    assert_eq!(typed.invoice, "NOPE9");
    let err = run_phase(&ctx, &request(Some(&["NOPE9", "NOPE8"]))).expect_err("several unknown");
    assert!(err.downcast_ref::<MissingRipsJsonError>().is_none());
    assert_eq!(
        err.to_string(),
        "none of the --invoices has a <F>.json in the folder"
    );
    // Known invoices still go through when others are unknown.
    let report = run_phase(&ctx, &request(Some(&["NOPE9", "FVS1"]))).expect("partial");
    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(fake.sent_invoices(), ["FVS1"]);
}

#[test]
fn configuration_and_worker_limits_are_checked_first() {
    let db = TestDb::new();
    let ctx = tessia::context::AppContext::for_tests(&db);
    let err = run_phase(&ctx, &request(None)).expect_err("missing contract");
    assert!(
        err.to_string().contains("minsalud.numero_contrato"),
        "{err}"
    );
    let ctx = context(&db, "", "CT-1");
    for workers in [0, 41] {
        let mut req = request(None);
        req.workers = Some(workers);
        let err = run_phase(&ctx, &req).expect_err("workers");
        assert_eq!(err.to_string(), "--workers must be between 1 and 40");
    }
    // Credentials are only needed when something is sent.
    seed(&ctx, "FVS1", "01", None);
    let report = run_phase(&ctx, &request(None)).expect("skip only");
    assert_eq!(report.outcomes.len(), 1);
    let (regimen, _, input, _) = xml_case("contributivo");
    seed(&ctx, "FVS2", &regimen, Some(&input));
    let err = run_phase(&ctx, &request(Some(&["FVS2"]))).expect_err("needs credentials");
    assert!(err.to_string().contains("minsalud.base_url"), "{err}");
    assert!(!err.to_string().contains("secret"));
    // The claim of the failed invoice was given back.
    assert!(state(&ctx, Phase::Cuv, "FVS2").is_none());
}

#[test]
fn the_lease_name_is_shared_with_fev_ajustar() {
    assert_eq!(
        lease_name("Convenio_1_2", "fvs-12"),
        "cuv:Convenio_1_2:FVS12"
    );
    assert_eq!(cuv::SKIP_OTHER_MACHINE, "EN_PROCESO_OTRA_MAQUINA");
}

#[test]
fn the_command_line_runs_generar_and_barrer() {
    use clap::Parser;
    use tessia::cli::{Cli, Commands};

    let db = TestDb::new();
    let (regimen, contrato, input, _) = xml_case("contributivo");
    let fake = FakeSispro::start(answers(&[("FVS1", 200, success_body("FVS1"))]));
    let ctx = context(&db, fake.url(), &contrato);
    seed(&ctx, "FVS1", &regimen, Some(&input));
    let parse = |args: &[&str]| {
        let argv: Vec<&str> = ["facturacion", "cuv"].iter().chain(args).copied().collect();
        match Cli::try_parse_from(argv).expect("parse").command {
            Commands::Cuv(args) => args,
            other => panic!("unexpected {other:?}"),
        }
    };

    cuv::run(
        &ctx,
        parse(&[
            "generar",
            "--cargue",
            CARGUE,
            "--invoices",
            "fvs-1",
            "--workers",
            "2",
        ]),
    )
    .expect("generar");
    assert_eq!(stored(&ctx, "FVS1").0, PhaseState::Done);
    publish(
        &ctx,
        "FVS1.txt",
        br#"{"ResultState":false,"CodigoUnicoValidacion":"x","RutaArchivos":"a\\FVS1"}"#,
    );
    cuv::run(&ctx, parse(&["barrer", "--cargue", CARGUE])).expect("barrer");
    assert!(fetch(&ctx, "FVS1.txt").starts_with(b"{\n  \"ResultState\": true"));

    let err = cuv::run(&ctx, parse(&["generar"])).expect_err("cargue required");
    assert!(err.to_string().contains("--cargue"), "{err}");
    let err = cuv::run(&ctx, parse(&["barrer"])).expect_err("cargue required");
    assert!(err.to_string().contains("--cargue"), "{err}");
}
