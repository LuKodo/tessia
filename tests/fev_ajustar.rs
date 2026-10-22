//! `fev ajustar` against the in-memory bucket and a throwaway Postgres.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::Parser;
use tessia::cli::{Cli, Commands};
use tessia::config::Settings;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::{Phase, PhaseState};
use tessia::fevxml::{build_xml_pdf_zip, first_xml_entry, read_entry};
use tessia::phases::common::PhaseRequest;
use tessia::phases::fev::{self, AjustarOp, cuv_lease_name};
use tessia::storage::{ListPage, MemoryStore, ObjectStore, StorageError, StorageResult};
use tessia::stores::proceso::ClaimItem;

const CARGUE: &str = "Convenio_21102026_21102026";
const CUCON: &str = "CUCON-SINT&<9>";

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fevxml/fevajuste/xml")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn zip_of(invoice: &str, xml: &[u8]) -> Vec<u8> {
    build_xml_pdf_zip(
        &format!("{invoice}.pdf"),
        b"%PDF-1.4 synthetic",
        &format!("{invoice}.xml"),
        xml,
        None,
    )
    .expect("zip")
}

fn publish(ctx: &AppContext, rel: &str, data: &[u8]) {
    ctx.artifacts()
        .publish_bytes(CARGUE, rel, data)
        .expect("publish");
}

fn fetch(ctx: &AppContext, rel: &str) -> Vec<u8> {
    ctx.artifacts().fetch_bytes(CARGUE, rel).expect("fetch")
}

fn sha_of(ctx: &AppContext, rel: &str) -> String {
    ctx.artefactos()
        .get(CARGUE, rel)
        .expect("index")
        .expect("row")
        .sha256
}

fn request() -> PhaseRequest {
    PhaseRequest::new(CARGUE)
}

fn state(ctx: &AppContext, phase: Phase, invoice: &str) -> Option<(PhaseState, String, i32)> {
    ctx.proceso()
        .load(phase, Some(&[invoice.to_owned()]))
        .expect("load")
        .remove(invoice)
        .map(|s| (s.state, s.motivo, s.intentos))
}

fn cucon() -> AjustarOp {
    AjustarOp::Cucon(CUCON.to_owned())
}

/// Cargue with: loose FEV1 (legacy contract), FEV2 inside XML/ zip (legacy),
/// FEV3 (other contract), FEV4 (no contract) and a CRC zip that must not be touched.
fn seed(ctx: &AppContext) {
    publish(ctx, "FEV1.xml", &fixture("contrato_22382.input.xml"));
    publish(
        ctx,
        "XML/FEV2_XML_PDF.zip",
        &zip_of("FEV2", &fixture("contrato_22382.input.xml")),
    );
    publish(ctx, "FEV3.xml", &fixture("contrato_otro.input.xml"));
    publish(ctx, "FEV4.xml", &fixture("sin_contrato.input.xml"));
    publish(ctx, "CRC/CRC_806_FEV1.zip", b"PK-not-touched");
}

#[test]
fn cucon_rewrites_loose_and_zipped_xml_and_records_each_invoice() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    seed(&ctx);
    let crc_before = sha_of(&ctx, "CRC/CRC_806_FEV1.zip");
    let other_before = sha_of(&ctx, "FEV3.xml");

    let run = fev::ajustar::ajustar(&ctx, &request(), &cucon()).expect("ajustar");

    assert_eq!(
        run.lines,
        [
            "  FEV1.xml modificado",
            "  FEV3.xml contrato_diferente",
            "  FEV4.xml sin_contrato",
            "  XML/FEV2_XML_PDF.zip:FEV2.xml modificado",
            "fev ajustar cucon: modificados=2 sin_contrato=1 contrato_diferente=1 (archivos a escribir=2)",
        ]
    );
    assert!(run.write_errors.is_empty());
    // Loose XML: byte-compatible with the Go golden.
    assert_eq!(fetch(&ctx, "FEV1.xml"), fixture("contrato_22382.cucon.xml"));
    // ZIP: other entries kept, the XML replaced and last.
    let zip = fetch(&ctx, "XML/FEV2_XML_PDF.zip");
    assert_eq!(
        first_xml_entry(&zip).expect("zip").expect("xml"),
        fixture("contrato_22382.cucon.xml")
    );
    assert_eq!(
        read_entry(&zip, "FEV2.pdf").expect("zip").expect("pdf"),
        b"%PDF-1.4 synthetic"
    );
    // Untouched artifacts keep their bytes.
    assert_eq!(sha_of(&ctx, "CRC/CRC_806_FEV1.zip"), crc_before);
    assert_eq!(sha_of(&ctx, "FEV3.xml"), other_before);
    // States: DONE with the counters each invoice fell in.
    for (invoice, reason) in [
        ("FEV1", "cucon: modificado"),
        ("FEV2", "cucon: modificado"),
        ("FEV3", "cucon: contrato_diferente"),
        ("FEV4", "cucon: sin_contrato"),
    ] {
        assert_eq!(
            state(&ctx, Phase::FevAjuste, invoice),
            Some((PhaseState::Done, reason.to_owned(), 1)),
            "{invoice}"
        );
    }
    assert_eq!(run.report.outcomes.len(), 4);
    assert!(run.report.skipped.is_empty());
    // Leases are released once the command ends.
    assert_eq!(
        ctx.leases()
            .holder(&cuv_lease_name(CARGUE, "FEV1"))
            .expect("holder"),
        None
    );
}

#[test]
fn the_xml_totals_of_rewritten_invoices_are_stored() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    seed(&ctx);

    fev::ajustar::ajustar(&ctx, &request(), &cucon()).expect("ajustar");

    let values = ctx
        .valor_xml()
        .stored_xml_values(&["FEV1".into(), "FEV2".into(), "FEV3".into()])
        .expect("values");
    let amount = |inv: &str| values.get(inv).and_then(|v| v.amount);
    assert_eq!(amount("FEV1"), Some("6500.00".parse().expect("dec")));
    assert_eq!(amount("FEV2"), Some("6500.00".parse().expect("dec")));
    assert!(!values.contains_key("FEV3"), "only rewritten invoices");
    let stamp = values["FEV2"].stamp.expect("stamp");
    assert_eq!(
        stamp.1,
        fetch(&ctx, "XML/FEV2_XML_PDF.zip").len() as i64,
        "the stamp is the rewritten ZIP"
    );
}

#[test]
fn a_second_run_finds_nothing_to_write() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    seed(&ctx);
    fev::ajustar::ajustar(&ctx, &request(), &cucon()).expect("first");
    let before = sha_of(&ctx, "FEV1.xml");

    let run = fev::ajustar::ajustar(&ctx, &request(), &cucon()).expect("second");

    assert_eq!(
        run.lines.last().map(String::as_str),
        Some(
            "fev ajustar cucon: modificados=0 sin_contrato=1 contrato_diferente=3 (archivos a escribir=0)"
        )
    );
    assert_eq!(sha_of(&ctx, "FEV1.xml"), before);
    assert_eq!(
        state(&ctx, Phase::FevAjuste, "FEV1"),
        Some((PhaseState::Done, "cucon: contrato_diferente".into(), 2)),
        "attempts continue from the stored row"
    );
}

#[test]
fn cuota_prints_warnings_and_is_idempotent() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    publish(&ctx, "FEV5.xml", &fixture("anticipo.input.xml"));
    publish(&ctx, "FEV6.xml", &fixture("contrato_otro.input.xml"));

    let run = fev::ajustar::ajustar(&ctx, &request(), &AjustarOp::Cuota).expect("ajustar");

    assert_eq!(
        run.lines,
        [
            "  FEV5.xml modificado",
            "    ADVERTENCIA FEV5.xml: PrepaidAmount=3000.0 no coincide con suma PaidAmount=3000.5",
            "  FEV6.xml sin_cambios",
            "fev ajustar cuota: procesados=2 modificados=1 errores=0 (archivos a escribir=1)",
        ]
    );
    assert_eq!(fetch(&ctx, "FEV5.xml"), fixture("anticipo.cuota.xml"));
    assert_eq!(
        state(&ctx, Phase::FevAjuste, "FEV5"),
        Some((PhaseState::Done, "cuota: modificado".into(), 1))
    );
    assert_eq!(
        state(&ctx, Phase::FevAjuste, "FEV6"),
        Some((PhaseState::Done, "cuota: sin_cambios".into(), 1))
    );
    let again = fev::ajustar::ajustar(&ctx, &request(), &AjustarOp::Cuota).expect("again");
    assert!(
        again
            .lines
            .last()
            .is_some_and(|l| l.ends_with("(archivos a escribir=0)")),
        "{:?}",
        again.lines
    );
}

#[test]
fn invoices_filter_limits_the_files_looked_at() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    seed(&ctx);
    let mut req = request();
    req.invoices = Some(vec!["FEV2".into()]);

    let run = fev::ajustar::ajustar(&ctx, &req, &cucon()).expect("ajustar");

    assert_eq!(
        run.lines,
        [
            "  XML/FEV2_XML_PDF.zip:FEV2.xml modificado",
            "fev ajustar cucon: modificados=1 sin_contrato=0 contrato_diferente=0 (archivos a escribir=1)",
        ]
    );
    assert_eq!(
        fetch(&ctx, "FEV1.xml"),
        fixture("contrato_22382.input.xml"),
        "FEV1 was not asked for"
    );
    assert_eq!(state(&ctx, Phase::FevAjuste, "FEV1"), None);
}

#[test]
fn a_corrupt_zip_fails_its_invoice_and_the_others_go_on() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    publish(&ctx, "XML/FEV7_XML_PDF.zip", b"this is not a zip");
    publish(&ctx, "FEV1.xml", &fixture("contrato_22382.input.xml"));

    let run = fev::ajustar::ajustar(&ctx, &request(), &AjustarOp::Cuota).expect("ajustar");

    assert!(
        run.lines
            .iter()
            .any(|l| l.starts_with("  XML/FEV7_XML_PDF.zip ERROR ")),
        "{:?}",
        run.lines
    );
    assert!(
        run.lines.last().is_some_and(
            |l| l.starts_with("fev ajustar cuota: procesados=2 modificados=1 errores=1")
        ),
        "{:?}",
        run.lines
    );
    let (st, motivo, _) = state(&ctx, Phase::FevAjuste, "FEV7").expect("state");
    assert_eq!(st, PhaseState::Failed);
    assert!(
        motivo.starts_with("ERROR XML/FEV7_XML_PDF.zip: "),
        "{motivo}"
    );
    assert_eq!(
        state(&ctx, Phase::FevAjuste, "FEV1").map(|s| s.0),
        Some(PhaseState::Done)
    );
}

#[test]
fn a_lease_held_by_another_machine_skips_that_invoice_only() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db).with_owner("machine-a:1:x");
    seed(&ctx);
    assert!(
        ctx.leases()
            .acquire(
                &cuv_lease_name(CARGUE, "FEV1"),
                "machine-b:2:y",
                Duration::from_secs(600)
            )
            .expect("lease")
    );

    let run = fev::ajustar::ajustar(&ctx, &request(), &cucon()).expect("ajustar");

    assert_eq!(
        run.report.skipped,
        [("FEV1".to_owned(), "EN_PROCESO_OTRA_MAQUINA".to_owned())]
    );
    assert_eq!(
        fetch(&ctx, "FEV1.xml"),
        fixture("contrato_22382.input.xml"),
        "not rewritten"
    );
    assert_eq!(state(&ctx, Phase::FevAjuste, "FEV1"), None);
    assert_eq!(
        first_xml_entry(&fetch(&ctx, "XML/FEV2_XML_PDF.zip"))
            .expect("zip")
            .expect("xml"),
        fixture("contrato_22382.cucon.xml"),
        "FEV2 was adjusted"
    );
}

#[test]
fn an_invoice_claimed_by_another_machine_is_not_rewritten() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db).with_owner("machine-a:1:x");
    seed(&ctx);
    let other = ctx.claim_options(CARGUE);
    let other = tessia::stores::proceso::ClaimOptions::new(
        "machine-b:2:y",
        Some(CARGUE),
        other.stale_after,
    );
    let claimed = ctx
        .proceso()
        .claim_available(Phase::FevAjuste, &[ClaimItem::new("FEV3")], &other)
        .expect("claim");
    assert_eq!(claimed.len(), 1);

    let run = fev::ajustar::ajustar(&ctx, &request(), &cucon()).expect("ajustar");

    assert!(
        run.lines.contains(
            &"warning: FEV3 is already in execution for phase fev_ajuste; skipping it".to_owned()
        ),
        "{:?}",
        run.lines
    );
    assert_eq!(
        run.report.skipped,
        [("FEV3".to_owned(), "EN_PROCESO_OTRA_MAQUINA".to_owned())]
    );
    assert_eq!(
        state(&ctx, Phase::FevAjuste, "FEV3").map(|s| s.0),
        Some(PhaseState::EnProgreso),
        "the other machine keeps its claim"
    );
    assert_eq!(run.report.outcomes.len(), 3);
}

/// Object store that refuses to store keys containing `fail_on` once armed.
struct FailingStore {
    inner: MemoryStore,
    fail_on: String,
    armed: AtomicBool,
}

impl ObjectStore for FailingStore {
    fn put_bytes(&self, key: &str, data: &[u8], content_type: &str) -> StorageResult<()> {
        if self.armed.load(Ordering::SeqCst) && key.contains(&self.fail_on) {
            return Err(StorageError::Failed("disk full".into()));
        }
        self.inner.put_bytes(key, data, content_type)
    }
    fn put_file(&self, key: &str, path: &Path, content_type: &str) -> StorageResult<()> {
        self.inner.put_file(key, path, content_type)
    }
    fn get_bytes(&self, key: &str) -> StorageResult<Vec<u8>> {
        self.inner.get_bytes(key)
    }
    fn get_to_file(&self, key: &str, dest: &Path) -> StorageResult<u64> {
        self.inner.get_to_file(key, dest)
    }
    fn exists(&self, key: &str) -> StorageResult<bool> {
        self.inner.exists(key)
    }
    fn delete(&self, key: &str) -> StorageResult<bool> {
        self.inner.delete(key)
    }
    fn list(&self, start: Option<&str>, limit: usize) -> StorageResult<ListPage> {
        self.inner.list(start, limit)
    }
}

#[test]
fn a_file_that_cannot_be_written_fails_its_invoice_and_the_rest_are_written() {
    let db = TestDb::new();
    let store = Arc::new(FailingStore {
        inner: MemoryStore::new(),
        fail_on: "FEV1.xml".into(),
        armed: AtomicBool::new(false),
    });
    let ctx = AppContext::for_tests(&db).with_objects(store.clone());
    publish(&ctx, "FEV1.xml", &fixture("contrato_22382.input.xml"));
    publish(&ctx, "FEV8.xml", &fixture("contrato_22382.input.xml"));
    store.armed.store(true, Ordering::SeqCst);

    let run = fev::ajustar::ajustar(&ctx, &request(), &cucon()).expect("ajustar");

    assert_eq!(run.write_errors.len(), 1, "{:?}", run.write_errors);
    let (st, motivo, _) = state(&ctx, Phase::FevAjuste, "FEV1").expect("state");
    assert_eq!(st, PhaseState::Failed);
    assert!(
        motivo.starts_with("cucon: modificado; ERROR write FEV1.xml: "),
        "{motivo}"
    );
    assert_eq!(
        state(&ctx, Phase::FevAjuste, "FEV8"),
        Some((PhaseState::Done, "cucon: modificado".into(), 1))
    );
    assert_eq!(fetch(&ctx, "FEV8.xml"), fixture("contrato_22382.cucon.xml"));
    let values = ctx
        .valor_xml()
        .stored_xml_values(&["FEV1".into(), "FEV8".into()])
        .expect("values");
    assert!(
        !values.contains_key("FEV1"),
        "a failed invoice stores no XML value"
    );
    assert!(values.contains_key("FEV8"));
}

fn parse_fev(args: &[&str]) -> fev::Args {
    let mut argv = vec!["facturacion", "fev"];
    argv.extend_from_slice(args);
    match Cli::try_parse_from(argv).expect("parse").command {
        Commands::Fev(args) => args,
        other => panic!("not fev: {other:?}"),
    }
}

#[test]
fn the_command_line_applies_cucon_from_the_flag_or_the_configuration() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    seed(&ctx);
    // No flag and no configuration: the error names the setting.
    let err = fev::run(
        &ctx,
        parse_fev(&["ajustar", "--cargue", CARGUE, "--op", "cucon"]),
    )
    .expect_err("missing cucon");
    assert!(err.to_string().contains("--cucon o"), "{err}");
    assert!(
        err.to_string().contains("minsalud.numero_contrato"),
        "{err}"
    );
    assert_eq!(fetch(&ctx, "FEV1.xml"), fixture("contrato_22382.input.xml"));

    // Configured contract.
    let configured = AppContext::for_tests(&db)
        .with_settings(Settings::default().with("minsalud.numero_contrato", " 5555 "));
    let configured = configured.with_objects(ctx.objects.clone());
    fev::run(
        &configured,
        parse_fev(&[
            "ajustar",
            "--cargue",
            CARGUE,
            "--op",
            "cucon",
            "--invoices",
            "fev-1",
        ]),
    )
    .expect("configured");
    let xml = String::from_utf8(fetch(&ctx, "FEV1.xml")).expect("utf8");
    assert!(xml.contains(">5555</Value>"), "{xml}");

    // The flag wins over the configuration.
    publish(&ctx, "FEV9.xml", &fixture("contrato_22382.input.xml"));
    fev::run(
        &configured,
        parse_fev(&[
            "ajustar",
            "--cargue",
            CARGUE,
            "--op",
            "cucon",
            "--cucon",
            "777",
            "--invoices",
            "FEV9",
        ]),
    )
    .expect("flag");
    let xml = String::from_utf8(fetch(&ctx, "FEV9.xml")).expect("utf8");
    assert!(xml.contains(">777</Value>"), "{xml}");
}

#[test]
fn op_is_required_and_validated() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    seed(&ctx);
    let err = fev::run(&ctx, parse_fev(&["ajustar", "--cargue", CARGUE])).expect_err("no op");
    assert_eq!(err.to_string(), "--op debe ser cucon o cuota");
    assert!(
        Cli::try_parse_from(["facturacion", "fev", "ajustar", "--op", "otra"]).is_err(),
        "unknown op"
    );
    assert_eq!(
        fetch(&ctx, "FEV1.xml"),
        fixture("contrato_22382.input.xml"),
        "nothing is touched"
    );
}
