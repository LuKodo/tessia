//! The phase end to end over a test database and an in-memory bucket: ZIP
//! artifacts, states, claims, leases, signatures and regeneration.

#[path = "fixtures/soporte/common.rs"]
mod soporte_common;

use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

use image::ImageFormat;
use soporte_common::*;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::{Phase, PhaseState};
use tessia::phases::common::PhaseRequest;
use tessia::phases::soporte::assets::{SHARED_CARGUE, signature_rel};
use tessia::phases::soporte::cpscan::{payload_rel, sidecar_rel};
use tessia::phases::soporte::render::merge::page_texts;
use tessia::phases::soporte::render::signature;
use tessia::phases::soporte::{Options, SoporteInput, SoporteRow, run_inputs};
use tessia::storage::{ListPage, MemoryStore, ObjectStore, StorageResult};
use tessia::stores::proceso::ClaimItem;

const CARGUE: &str = "Convenio_21102026_21102026";
const ZIP: &str = "CRC/CRC_806011811_FEV1.zip";

fn row(ssc: &str, ceco: &str) -> SoporteRow {
    SoporteRow {
        ssc: format!("SSC{ssc}"),
        point_of_sale: ceco.into(),
        authorization: "AUT-1".into(),
        doc_type: "CC".into(),
        doc_number: "123".into(),
        beneficiary: "ANA PEREZ".into(),
        date: "2026-10-05 10:00:00".into(),
        article: "ACETAMINOFEN".into(),
        quantity: "2".into(),
    }
}

fn input(invoice: &str, ssc: &str, ceco: &str) -> SoporteInput {
    SoporteInput {
        invoice: invoice.into(),
        rows: vec![row(ssc, ceco)],
    }
}

fn publish(ctx: &AppContext, rel: &str, data: &[u8]) {
    ctx.artifacts()
        .publish_bytes(CARGUE, rel, data)
        .expect("publish");
}

/// CPScan files of an APP invoice: sidecar + formula image.
fn publish_app(ctx: &AppContext, invoice: &str) {
    let sidecar = r#"{"version":1,"ssc":"10","documento":true,"origen":"APP","archivo":"formula-10.jpg","ext":"jpg"}"#;
    publish(ctx, &sidecar_rel(invoice), sidecar.as_bytes());
    publish(
        ctx,
        &payload_rel(invoice, "jpg"),
        &rgb_image(200, 120, ImageFormat::Jpeg),
    );
}

fn publish_nothing(ctx: &AppContext, invoice: &str) {
    let sidecar = r#"{"version":1,"ssc":"10","documento":true,"origen":"APP","archivo":""}"#;
    publish(ctx, &sidecar_rel(invoice), sidecar.as_bytes());
}

fn publish_invoice_pdf(ctx: &AppContext, invoice: &str) {
    publish(ctx, &format!("{invoice}.pdf"), &text_pdf("FACTURA", 1));
}

fn run(ctx: &AppContext, inputs: Vec<SoporteInput>) -> tessia::phases::soporte::SoporteRun {
    run_inputs(ctx, &PhaseRequest::new(CARGUE), Options::default(), inputs).expect("run")
}

fn state(ctx: &AppContext, invoice: &str) -> (PhaseState, String, i32) {
    let loaded = ctx
        .proceso()
        .load(Phase::Soporte, Some(&[invoice.to_owned()]))
        .expect("load");
    let s = loaded.get(invoice).expect("state");
    (s.state, s.motivo.clone(), s.intentos)
}

fn zip_pdf(ctx: &AppContext, rel: &str) -> Vec<u8> {
    let bytes = ctx.artifacts().fetch_bytes(CARGUE, rel).expect("zip");
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("open");
    assert_eq!(archive.len(), 1);
    let mut entry = archive.by_index(0).expect("entry");
    assert_eq!(entry.compression(), zip::CompressionMethod::Deflated);
    let name = entry.name().expect("name").into_owned();
    assert!(
        name.starts_with("CRC_806011811_") && name.ends_with(".pdf"),
        "{name}"
    );
    let mut pdf = Vec::new();
    entry.read_to_end(&mut pdf).expect("read");
    pdf
}

fn setup() -> (TestDb, AppContext) {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    (db, ctx)
}

#[test]
fn an_app_invoice_publishes_the_zip_and_is_done() {
    let (_db, ctx) = setup();
    publish_app(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    let result = run(&ctx, vec![input("FEV1", "10", "CENTRO")]);
    assert_eq!(result.lines, ["  FEV1 OK rama=APP"]);
    assert_eq!((result.ok, result.existing, result.failed), (1, 0, 0));
    assert_eq!(result.summary_line(), "soporte: ok=1 existing=0 failed=0");
    assert!(result.errors.is_empty());
    assert_eq!(result.report.counters().done, 1);
    assert_eq!(state(&ctx, "FEV1"), (PhaseState::Done, String::new(), 1));
    let pdf = zip_pdf(&ctx, ZIP);
    assert_eq!(page_texts(&pdf).expect("texts"), ["", "FACTURA page 1"]);
    let artifact = ctx
        .artefactos()
        .get(CARGUE, ZIP)
        .expect("index")
        .expect("row");
    assert!(artifact.size > 0);
}

#[test]
fn an_existing_zip_is_kept_and_regenerar_replaces_it() {
    let (_db, ctx) = setup();
    publish_app(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    run(&ctx, vec![input("FEV1", "10", "CENTRO")]);
    let before = ctx
        .artefactos()
        .get(CARGUE, ZIP)
        .expect("index")
        .expect("row");

    let again = run(&ctx, vec![input("FEV1", "10", "CENTRO")]);
    assert_eq!(again.lines, ["  FEV1 YA_EXISTE"]);
    assert_eq!((again.ok, again.existing, again.failed), (0, 1, 0));
    assert_eq!(state(&ctx, "FEV1"), (PhaseState::Done, String::new(), 2));
    assert_eq!(
        ctx.artefactos()
            .get(CARGUE, ZIP)
            .expect("index")
            .expect("row")
            .sha256,
        before.sha256
    );

    publish(&ctx, "FEV1.pdf", &text_pdf("FACTURA CORREGIDA", 1));
    let regenerated = run_inputs(
        &ctx,
        &PhaseRequest::new(CARGUE),
        Options { regenerate: true },
        vec![input("FEV1", "10", "CENTRO")],
    )
    .expect("run");
    assert_eq!(
        regenerated.lines,
        ["  FEV1 OK rama=APP regenerado (ZIP reemplazado)"]
    );
    assert_eq!(regenerated.ok, 1);
    assert_ne!(
        ctx.artefactos()
            .get(CARGUE, ZIP)
            .expect("index")
            .expect("row")
            .sha256,
        before.sha256,
        "the new ZIP replaced the old one"
    );
    assert_eq!(
        page_texts(&zip_pdf(&ctx, ZIP)).expect("texts")[1],
        "FACTURA CORREGIDA page 1"
    );
}

#[test]
fn a_failed_regeneration_keeps_the_previous_zip() {
    let (_db, ctx) = setup();
    publish_app(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    run(&ctx, vec![input("FEV1", "10", "CENTRO")]);
    let before = ctx
        .artefactos()
        .get(CARGUE, ZIP)
        .expect("index")
        .expect("row");
    ctx.artifacts().delete(CARGUE, "FEV1.pdf").expect("delete");

    let result = run_inputs(
        &ctx,
        &PhaseRequest::new(CARGUE),
        Options { regenerate: true },
        vec![input("FEV1", "10", "CENTRO")],
    )
    .expect("run");
    assert_eq!(result.lines.len(), 1);
    assert!(
        result.lines[0].starts_with("  FEV1 FACTURA_PDF_FALTA (falta FEV1.pdf")
            && result.lines[0].ends_with("; ZIP anterior conservado)"),
        "{}",
        result.lines[0]
    );
    assert_eq!(result.failed, 1);
    let (st, motivo, _) = state(&ctx, "FEV1");
    assert_eq!(st, PhaseState::Failed);
    assert!(
        motivo.starts_with("FACTURA_PDF_FALTA: falta FEV1.pdf")
            && motivo.ends_with("ZIP anterior conservado")
    );
    assert_eq!(
        ctx.artefactos()
            .get(CARGUE, ZIP)
            .expect("index")
            .expect("row")
            .sha256,
        before.sha256
    );
}

#[test]
fn the_nit_comes_from_zonaser_before_company() {
    let (_db, ctx) = setup();
    let settings = ctx
        .settings
        .clone()
        .with("zonaser.organization_nit", " 900123 ");
    let ctx = ctx.with_settings(settings);
    publish_app(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    run(&ctx, vec![input("FEV1", "10", "CENTRO")]);
    assert!(
        ctx.artifacts()
            .is_published(CARGUE, "CRC/CRC_900123_FEV1.zip")
            .expect("index")
    );
    assert!(!ctx.artifacts().is_published(CARGUE, ZIP).expect("index"));
}

#[test]
fn result_codes_map_to_states_and_reasons() {
    let (_db, ctx) = setup();
    // SIN_SSC: no SSC in the dataset.
    let mut no_ssc = input("FEV1", "", "CENTRO");
    no_ssc.rows[0].ssc = String::new();
    // FACTURA_PDF_FALTA: CPScan fine, invoice missing.
    publish_app(&ctx, "FEV2");
    // CPSCAN_NO_DESCARGADO: invoice there, no sidecar.
    publish_invoice_pdf(&ctx, "FEV3");
    // PC_SIN_URL.
    let sidecar = r#"{"version":1,"ssc":"10","documento":true,"origen":"PC","archivo":""}"#;
    publish(&ctx, &sidecar_rel("FEV4"), sidecar.as_bytes());
    publish_invoice_pdf(&ctx, "FEV4");
    // Mandatory deposit without support.
    publish_nothing(&ctx, "FEV5");
    publish_invoice_pdf(&ctx, "FEV5");
    // Non mandatory deposit without support.
    publish_nothing(&ctx, "FEV6");
    publish_invoice_pdf(&ctx, "FEV6");
    // Corrupt sidecar.
    publish(&ctx, &sidecar_rel("FEV7"), b"{not json");
    publish_invoice_pdf(&ctx, "FEV7");

    let result = run(
        &ctx,
        vec![
            no_ssc,
            input("FEV2", "10", "CENTRO"),
            input("FEV3", "10", "CENTRO"),
            input("FEV4", "10", "CENTRO"),
            input("FEV5", "10", "BODEGA 1"),
            input("FEV6", "10", "BLOC PORT"),
            input("FEV7", "10", "CENTRO"),
            input("bad/1", "10", "CENTRO"),
        ],
    );
    let get = |inv: &str| state(&ctx, inv);
    assert_eq!(
        get("FEV1"),
        (PhaseState::RevisionManual, "SIN_SSC".into(), 1)
    );
    assert_eq!(get("FEV2").0, PhaseState::Failed);
    assert!(
        get("FEV2")
            .1
            .starts_with("FACTURA_PDF_FALTA: falta FEV2.pdf")
    );
    assert!(
        get("FEV3")
            .1
            .starts_with("CPSCAN_NO_DESCARGADO: falta CPSCAN/FEV3.cpscan.json")
    );
    assert_eq!(
        get("FEV4"),
        (PhaseState::RevisionManual, "PC_SIN_URL".into(), 1)
    );
    assert_eq!(
        get("FEV5").1,
        "SOPORTE_OBLIGATORIO_NO_DISPONIBLE: CPScan sin soporte para BODEGA 1"
    );
    assert!(
        get("FEV6")
            .1
            .starts_with("FORMATO_DEPOSITO_NO_DISPONIBLE: CPScan sin soporte: el soporte manual")
    );
    assert_eq!(get("FEV7").0, PhaseState::Failed);
    assert!(
        get("FEV7")
            .1
            .starts_with("CPSCAN_NO_DESCARGADO: cpscan: CPSCAN/FEV7.cpscan.json: sidecar invalido"),
        "{}",
        get("FEV7").1
    );
    assert_eq!(
        get("bad/1"),
        (
            PhaseState::Failed,
            "FACTURA_INVALIDA: invalid invoice number".into(),
            1
        )
    );
    assert_eq!(result.ok, 0);
    assert_eq!(result.failed, 8);
    assert_eq!(result.summary_line(), "soporte: ok=0 existing=0 failed=8");
}

#[test]
fn revision_manual_is_skipped_unless_the_invoice_is_asked_for() {
    let (_db, ctx) = setup();
    let mut no_ssc = input("FEV1", "", "CENTRO");
    no_ssc.rows[0].ssc = String::new();
    run(&ctx, vec![no_ssc.clone()]);
    let skipped = run(&ctx, vec![no_ssc.clone()]);
    assert_eq!(
        skipped.lines,
        ["  FEV1 REVISION_MANUAL: soporte REVISION_MANUAL: SIN_SSC (reprocess with --invoices)"]
    );
    assert_eq!(
        skipped.report.skipped,
        [("FEV1".to_string(), "REVISION_MANUAL".to_string())]
    );
    assert_eq!(state(&ctx, "FEV1").2, 1, "not retried");

    let mut request = PhaseRequest::new(CARGUE);
    request.invoices = Some(vec!["FEV1".into()]);
    let forced = run_inputs(&ctx, &request, Options::default(), vec![no_ssc]).expect("run");
    assert_eq!(forced.failed, 1);
    assert_eq!(state(&ctx, "FEV1").2, 2);
}

#[test]
fn failed_invoices_are_retried_and_fixed_by_the_missing_inputs() {
    let (_db, ctx) = setup();
    publish_app(&ctx, "FEV1");
    let first = run(&ctx, vec![input("FEV1", "10", "CENTRO")]);
    assert_eq!(first.failed, 1);
    publish_invoice_pdf(&ctx, "FEV1");
    let second = run(&ctx, vec![input("FEV1", "10", "CENTRO")]);
    assert_eq!(second.ok, 1);
    assert_eq!(state(&ctx, "FEV1"), (PhaseState::Done, String::new(), 2));
}

#[test]
fn the_manual_support_generates_and_shares_the_patient_signature() {
    let (_db, ctx) = setup();
    publish_nothing(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    let result = run(&ctx, vec![input("FEV1", "10", "CENTRO")]);
    assert_eq!(
        result.lines,
        [
            "  FEV1 OK rama=MANUAL (soporte manual: CPScan sin soporte) [firma generada desde el nombre del paciente]"
        ]
    );
    assert_eq!(
        state(&ctx, "FEV1").1,
        "firma generada desde el nombre del paciente"
    );
    let pdf = zip_pdf(&ctx, ZIP);
    assert!(page_texts(&pdf).expect("texts")[0].contains("CONSTANCIA DE ENTREGA"));
    // The signature is a shared artifact under the reserved cargue id, with a flat key.
    let rel = signature_rel("CC123.png");
    assert_eq!(rel, "firmas/CC123.png");
    let saved = ctx
        .artifacts()
        .fetch_bytes(SHARED_CARGUE, &rel)
        .expect("shared signature");
    assert_eq!(&saved[1..4], b"PNG");
    assert!(
        ctx.objects
            .exists("_shared__firmas__CC123.png")
            .expect("key")
    );

    // A second invoice of the same patient reuses it and does not overwrite it.
    publish_nothing(&ctx, "FEV9");
    publish_invoice_pdf(&ctx, "FEV9");
    let result = run(&ctx, vec![input("FEV9", "10", "CENTRO")]);
    assert_eq!(
        result.lines,
        ["  FEV9 OK rama=MANUAL (soporte manual: CPScan sin soporte)"]
    );
    assert_eq!(
        ctx.artifacts()
            .fetch_bytes(SHARED_CARGUE, &rel)
            .expect("signature"),
        saved
    );
}

#[test]
fn an_existing_signature_is_used_as_is_and_an_invalid_one_is_not_replaced() {
    let (_db, ctx) = setup();
    let rel = signature_rel("CC123.png");
    let mine = signature::render("OTRO NOMBRE").expect("signature");
    ctx.artifacts()
        .publish_bytes(SHARED_CARGUE, &rel, &mine)
        .expect("publish");
    publish_nothing(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    assert_eq!(run(&ctx, vec![input("FEV1", "10", "CENTRO")]).ok, 1);
    assert_eq!(
        ctx.artifacts()
            .fetch_bytes(SHARED_CARGUE, &rel)
            .expect("signature"),
        mine
    );

    // A file under 10 bytes is invalid: a signature is drawn but the file is kept.
    ctx.artifacts()
        .publish_bytes(SHARED_CARGUE, &rel, b"tiny")
        .expect("publish");
    publish_nothing(&ctx, "FEV2");
    publish_invoice_pdf(&ctx, "FEV2");
    let result = run(&ctx, vec![input("FEV2", "10", "CENTRO")]);
    assert!(
        result.lines[0].contains(
            "firma generada sin guardar (el archivo existente no es valido y no se reemplaza)"
        ),
        "{:?}",
        result.lines
    );
    assert_eq!(
        ctx.artifacts()
            .fetch_bytes(SHARED_CARGUE, &rel)
            .expect("signature"),
        b"tiny"
    );
}

#[test]
fn unsafe_document_numbers_are_drawn_but_not_saved() {
    let (_db, ctx) = setup();
    publish_nothing(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    let mut odd = input("FEV1", "10", "CENTRO");
    odd.rows[0].doc_number = "../x".into();
    let result = run(&ctx, vec![odd]);
    assert!(result.lines[0].contains(
        "firma generada sin guardar (tipo/numero de documento no validos como nombre de archivo)"
    ));
    assert!(
        ctx.artifacts()
            .list(SHARED_CARGUE)
            .expect("list")
            .is_empty()
    );
}

#[test]
fn the_signed_delivery_page_of_cpscan_supports_needs_the_flag_and_an_existing_file() {
    let (_db, ctx) = setup();
    publish_app(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    let sig = signature::render("ANA PEREZ").expect("signature");
    ctx.artifacts()
        .publish_bytes(SHARED_CARGUE, &signature_rel("CC123.png"), &sig)
        .expect("publish");

    // Flag off (default): formula + invoice.
    run(&ctx, vec![input("FEV1", "10", "CENTRO")]);
    assert_eq!(page_texts(&zip_pdf(&ctx, ZIP)).expect("texts").len(), 2);

    // Flag on, file present: formula + delivery + invoice.
    let on = ctx.clone().with_settings(
        ctx.settings
            .clone()
            .with("soporte.signatures_enabled", "TRUE"),
    );
    let result = run_inputs(
        &on,
        &PhaseRequest::new(CARGUE),
        Options { regenerate: true },
        vec![input("FEV1", "10", "CENTRO")],
    )
    .expect("run");
    assert_eq!(result.ok, 1);
    let texts = page_texts(&zip_pdf(&on, ZIP)).expect("texts");
    assert_eq!(texts.len(), 3);
    assert!(texts[1].contains("CONSTANCIA DE ENTREGA"));

    // Flag on, no file: the signature is never generated for CPScan supports.
    let mut other = input("FEV1", "10", "CENTRO");
    other.rows[0].doc_number = "999".into();
    run_inputs(
        &on,
        &PhaseRequest::new(CARGUE),
        Options { regenerate: true },
        vec![other],
    )
    .expect("run");
    assert_eq!(page_texts(&zip_pdf(&on, ZIP)).expect("texts").len(), 2);
    assert!(
        !on.artifacts()
            .is_published(SHARED_CARGUE, &signature_rel("CC999.png"))
            .expect("index")
    );
}

#[test]
fn the_logo_comes_from_the_shared_artifacts_then_paths_reportes_then_the_embedded_one() {
    let (_db, ctx) = setup();
    publish_nothing(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    assert_eq!(
        run(&ctx, vec![input("FEV1", "10", "CENTRO")]).ok,
        1,
        "embedded placeholder"
    );

    // A broken shared logo only fails the invoices that render a delivery page.
    ctx.artifacts()
        .publish_bytes(SHARED_CARGUE, "reportes/logo.png", b"not an image")
        .expect("publish");
    publish_nothing(&ctx, "FEV2");
    publish_invoice_pdf(&ctx, "FEV2");
    publish_app(&ctx, "FEV3");
    publish_invoice_pdf(&ctx, "FEV3");
    let result = run(
        &ctx,
        vec![input("FEV2", "10", "CENTRO"), input("FEV3", "10", "CENTRO")],
    );
    assert_eq!((result.ok, result.failed), (1, 1), "{:?}", result.lines);
    assert!(
        state(&ctx, "FEV2")
            .1
            .contains("delivery logo is not a PNG or JPEG image")
    );

    // A valid local logo is used when the shared one is gone.
    ctx.artifacts()
        .delete(SHARED_CARGUE, "reportes/logo.png")
        .expect("delete");
    let dir = tempfile::tempdir().expect("tmp");
    std::fs::write(
        dir.path().join("logo.png"),
        rgb_image(120, 40, ImageFormat::Png),
    )
    .expect("write");
    let local = ctx.clone().with_settings(
        ctx.settings
            .clone()
            .with("paths.reportes", &dir.path().to_string_lossy()),
    );
    let result = run(&local, vec![input("FEV2", "10", "CENTRO")]);
    assert_eq!(result.ok, 1, "{:?}", result.lines);
    assert!(Path::new(dir.path()).exists());
}

#[test]
fn an_invoice_claimed_by_another_machine_is_skipped_and_left_alone() {
    let (_db, ctx) = setup();
    publish_app(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    let other = ctx.clone().with_owner("otra-maquina:1:x");
    let held = other
        .proceso()
        .claim_available(
            Phase::Soporte,
            &[ClaimItem::new("FEV1")],
            &other.claim_options(CARGUE),
        )
        .expect("claim");
    assert_eq!(held.len(), 1);

    let result = run(
        &ctx,
        vec![input("FEV1", "10", "CENTRO"), input("FEV2", "10", "CENTRO")],
    );
    assert_eq!(result.lines[0], "  FEV1 EN_PROCESO_OTRA_MAQUINA");
    assert_eq!(
        result.report.skipped,
        [("FEV1".to_string(), "EN_PROCESO_OTRA_MAQUINA".to_string())]
    );
    assert_eq!(result.report.outcomes.len(), 1, "FEV2 was processed");
    assert!(!ctx.artifacts().is_published(CARGUE, ZIP).expect("index"));
    assert_eq!(state(&ctx, "FEV1").0, PhaseState::EnProgreso);
}

#[test]
fn a_leased_invoice_gives_its_claim_back() {
    let (_db, ctx) = setup();
    publish_app(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    let lease = format!("soporte:{CARGUE}:FEV1");
    assert!(
        ctx.leases()
            .acquire(
                &lease,
                "otra-maquina:1:x",
                std::time::Duration::from_secs(600)
            )
            .expect("lease")
    );

    let result = run(&ctx, vec![input("FEV1", "10", "CENTRO")]);
    assert_eq!(result.lines, ["  FEV1 EN_PROCESO_OTRA_MAQUINA"]);
    assert_eq!(result.ok, 0);
    assert!(!ctx.artifacts().is_published(CARGUE, ZIP).expect("index"));
    let loaded = ctx
        .proceso()
        .load(Phase::Soporte, Some(&["FEV1".to_owned()]))
        .expect("load");
    assert!(loaded.is_empty(), "the claim was released: no row remains");

    ctx.leases()
        .release(&lease, "otra-maquina:1:x")
        .expect("release");
    assert_eq!(run(&ctx, vec![input("FEV1", "10", "CENTRO")]).ok, 1);
    assert_eq!(
        ctx.leases().holder(&lease).expect("holder"),
        None,
        "the guard releases the lease"
    );
}

/// A bucket that refuses to store the CRC ZIPs.
struct NoZips(MemoryStore);

impl ObjectStore for NoZips {
    fn put_bytes(&self, key: &str, data: &[u8], content_type: &str) -> StorageResult<()> {
        if key.contains("__CRC__") {
            return Err(tessia::storage::StorageError::Failed("disco lleno".into()));
        }
        self.0.put_bytes(key, data, content_type)
    }
    fn put_file(&self, key: &str, path: &Path, content_type: &str) -> StorageResult<()> {
        self.0.put_file(key, path, content_type)
    }
    fn get_bytes(&self, key: &str) -> StorageResult<Vec<u8>> {
        self.0.get_bytes(key)
    }
    fn get_to_file(&self, key: &str, dest: &Path) -> StorageResult<u64> {
        self.0.get_to_file(key, dest)
    }
    fn exists(&self, key: &str) -> StorageResult<bool> {
        self.0.exists(key)
    }
    fn delete(&self, key: &str) -> StorageResult<bool> {
        self.0.delete(key)
    }
    fn list(&self, start: Option<&str>, limit: usize) -> StorageResult<ListPage> {
        self.0.list(start, limit)
    }
}

#[test]
fn a_bucket_failure_is_reported_and_the_claim_released_for_a_retry() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db).with_objects(Arc::new(NoZips(MemoryStore::new())));
    publish_app(&ctx, "FEV1");
    publish_invoice_pdf(&ctx, "FEV1");
    let result = run(&ctx, vec![input("FEV1", "10", "CENTRO")]);
    assert_eq!(result.errors.len(), 1);
    assert!(
        result.errors[0].starts_with("FEV1: ") && result.errors[0].contains("disco lleno"),
        "{:?}",
        result.errors
    );
    assert!(result.lines[0].starts_with("  FEV1 ERROR: "));
    assert!(result.report.outcomes.is_empty());
    let loaded = ctx
        .proceso()
        .load(Phase::Soporte, Some(&["FEV1".to_owned()]))
        .expect("load");
    assert!(loaded.is_empty());
}

#[test]
fn bad_requests_are_errors() {
    let (_db, ctx) = setup();
    let mut request = PhaseRequest::new(CARGUE);
    request.workers = Some(36);
    let err = run_inputs(&ctx, &request, Options::default(), vec![]).expect_err("workers");
    assert_eq!(err.to_string(), "--workers must be between 1 and 35");
    request.workers = Some(0);
    assert!(run_inputs(&ctx, &request, Options::default(), vec![]).is_err());
    request.workers = Some(35);
    assert!(run_inputs(&ctx, &request, Options::default(), vec![]).is_ok());
    request.invoices = Some(vec!["FEV404".into()]);
    let err = run_inputs(
        &ctx,
        &request,
        Options::default(),
        vec![input("FEV1", "10", "C")],
    )
    .expect_err("not in dataset");
    assert_eq!(
        err.to_string(),
        "invoice FEV404 is not in the dataset of the load"
    );
}

#[test]
fn many_invoices_run_in_parallel_with_one_result_each() {
    let (_db, ctx) = setup();
    let inputs: Vec<SoporteInput> = (1..=12)
        .map(|i| input(&format!("FEV{i}"), "10", "CENTRO"))
        .collect();
    for i in 1..=12 {
        publish_app(&ctx, &format!("FEV{i}"));
        publish_invoice_pdf(&ctx, &format!("FEV{i}"));
    }
    let mut request = PhaseRequest::new(CARGUE);
    request.workers = Some(5);
    let result = run_inputs(&ctx, &request, Options::default(), inputs).expect("run");
    assert_eq!((result.ok, result.report.outcomes.len()), (12, 12));
    for i in 1..=12 {
        assert!(
            ctx.artifacts()
                .is_published(CARGUE, &format!("CRC/CRC_806011811_FEV{i}.zip"))
                .expect("index")
        );
    }
}
