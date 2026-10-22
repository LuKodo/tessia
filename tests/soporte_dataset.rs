//! `soporte generar` reading the invoices from the dataset store.

#[path = "fixtures/soporte/common.rs"]
mod soporte_common;

use soporte_common::*;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::dataset::test_support::{DatasetBuilder, InvoiceBuilder, sample_line};
use tessia::domain::{Phase, PhaseState};
use tessia::phases::common::{CommonArgs, PhaseRequest};
use tessia::phases::soporte::render::merge::page_texts;
use tessia::phases::soporte::{self, cli};
use tessia::stores::dataset::test_support::seed_cargue;

const LOAD: &str = "Convenio_21102026_21102026";

fn publish(ctx: &AppContext, rel: &str, data: &[u8]) {
    ctx.artifacts()
        .publish_bytes(LOAD, rel, data)
        .expect("publish");
}

fn publish_nothing(ctx: &AppContext, invoice: &str) {
    let sidecar = r#"{"version":1,"ssc":"10","documento":true,"origen":"APP","archivo":""}"#;
    publish(
        ctx,
        &format!("CPSCAN/{invoice}.cpscan.json"),
        sidecar.as_bytes(),
    );
    publish(ctx, &format!("{invoice}.pdf"), &text_pdf("FACTURA", 1));
}

fn line(invoice: &str, n: i32, ssc: &str, article: &str) -> tessia::domain::dataset::Line {
    let mut l = sample_line(invoice, n, "10");
    l.ssc = ssc.into();
    l.point_of_sale = " centro  medico ".into();
    l.authorization = format!("AUT{n}");
    l.article = article.into();
    l.date = "2026-10-21 08:30:00".into();
    l
}

fn setup() -> (TestDb, AppContext) {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    let seq = seed_cargue(&db.pool(), LOAD, 'a');
    let ds = DatasetBuilder::new(seq)
        .load(LOAD)
        .invoice(
            InvoiceBuilder::new("FEV1")
                .with_line(line("FEV1", 1, "SSC10", "ARTICULO UNO"))
                .with_line(line("FEV1", 2, "10", "ARTICULO DOS"))
                .with_line(line("FEV1", 3, "SSC11", "ARTICULO OTRO SSC")),
        )
        .invoice(InvoiceBuilder::new("FEV2").with_line({
            let mut other_patient = line("FEV2", 1, "SSC20", "ARTICULO X");
            other_patient.doc_number = "2000000002".into();
            other_patient
        }))
        .excluded("FEV3", "VALOR_SUPERA_MONTO")
        .build();
    ctx.dataset().replace(&ds, false).expect("replace");
    (db, ctx)
}

#[test]
fn run_phase_builds_the_support_of_every_prepared_invoice_from_the_dataset() {
    let (_db, ctx) = setup();
    publish_nothing(&ctx, "FEV1");
    publish_nothing(&ctx, "FEV2");
    let report = soporte::run_phase(&ctx, &PhaseRequest::new(LOAD)).expect("run");
    let invoices: Vec<&str> = report.outcomes.iter().map(|o| o.invoice.as_str()).collect();
    assert_eq!(
        invoices,
        ["FEV1", "FEV2"],
        "excluded invoices are not part of the dataset invoices"
    );
    assert_eq!(report.counters().done, 2);

    // The delivery page lists the first SSC's lines only; the other SSC is noted.
    let zip = ctx
        .artifacts()
        .fetch_bytes(LOAD, "CRC/CRC_806011811_FEV1.zip")
        .expect("zip");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).expect("open");
    let mut pdf = Vec::new();
    std::io::Read::read_to_end(&mut archive.by_index(0).expect("entry"), &mut pdf).expect("read");
    let texts = page_texts(&pdf).expect("texts");
    assert!(texts[0].contains("ARTICULO UNO") && texts[0].contains("ARTICULO DOS"));
    assert!(!texts[0].contains("ARTICULO OTRO SSC"));
    assert!(
        texts[0].contains("21/10/2026"),
        "date from the first 10 chars"
    );
    assert!(texts[0].contains("PACIENTE DE PRUEBA TIPO DOC: CC  NUM DOC: 1000000001"));

    let state = ctx
        .proceso()
        .load(Phase::Soporte, Some(&["FEV1".to_owned()]))
        .expect("load");
    let s = &state["FEV1"];
    assert_eq!(s.state, PhaseState::Done);
    assert_eq!(
        s.motivo,
        "firma generada desde el nombre del paciente; entrega: 1 linea(s) de otro SSC no se listaron (solo las del SSC 10)"
    );
    assert_eq!(s.cargue_id.as_deref(), Some(LOAD));
}

#[test]
fn only_the_requested_invoices_are_processed_and_unknown_ones_are_errors() {
    let (_db, ctx) = setup();
    publish_nothing(&ctx, "FEV2");
    let mut req = PhaseRequest::new(LOAD);
    req.invoices = Some(vec!["fev-2".into()]);
    let report = soporte::run_phase(&ctx, &req).expect("run");
    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(report.outcomes[0].invoice, "FEV2");
    assert!(
        !ctx.artifacts()
            .is_published(LOAD, "CRC/CRC_806011811_FEV1.zip")
            .expect("index")
    );

    // An excluded invoice is not in the dataset of the load.
    req.invoices = Some(vec!["FEV3".into()]);
    let err = soporte::run_phase(&ctx, &req).expect_err("excluded");
    assert_eq!(
        err.to_string(),
        "invoice FEV3 is not in the dataset of the load"
    );
}

#[test]
fn a_cargue_without_a_dataset_is_an_error() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    let err = soporte::run_phase(&ctx, &PhaseRequest::new("Convenio_01012026_01012026"))
        .expect_err("no dataset");
    assert_eq!(
        err.to_string(),
        "el cargue Convenio_01012026_01012026 no tiene un dataset preparado"
    );
}

#[test]
fn the_command_runs_the_same_work_and_honours_regenerar() {
    let (_db, ctx) = setup();
    publish_nothing(&ctx, "FEV2");
    let args = |regenerar: bool| cli::Args {
        command: cli::Command::Generar(cli::GenerarArgs {
            common: CommonArgs {
                cargue: Some(LOAD.into()),
                invoices: vec!["FEV2".into()],
                workers: Some(2),
            },
            regenerar,
        }),
    };
    soporte::run(&ctx, args(false)).expect("first run");
    let first = ctx
        .artefactos()
        .get(LOAD, "CRC/CRC_806011811_FEV2.zip")
        .expect("index")
        .expect("row");
    publish(&ctx, "FEV2.pdf", &text_pdf("FACTURA CORREGIDA", 1));
    soporte::run(&ctx, args(false)).expect("kept");
    assert_eq!(
        ctx.artefactos()
            .get(LOAD, "CRC/CRC_806011811_FEV2.zip")
            .expect("index")
            .expect("row")
            .sha256,
        first.sha256
    );
    soporte::run(&ctx, args(true)).expect("regenerated");
    assert_ne!(
        ctx.artefactos()
            .get(LOAD, "CRC/CRC_806011811_FEV2.zip")
            .expect("index")
            .expect("row")
            .sha256,
        first.sha256
    );
}

#[test]
fn the_command_needs_a_cargue() {
    let (_db, ctx) = setup();
    let args = cli::Args {
        command: cli::Command::Generar(cli::GenerarArgs {
            common: CommonArgs::default(),
            regenerar: false,
        }),
    };
    let err = soporte::run(&ctx, args).expect_err("no cargue");
    assert!(err.to_string().contains("falta --cargue"));
}
