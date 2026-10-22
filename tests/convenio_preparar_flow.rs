//! `convenio preparar` end to end: a local CSV or an existing cargue, the
//! Redis NT fake, a fake MIPRES service and a throwaway Postgres database.
//! No real system is contacted.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tessia::config::Settings;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::cargue::LoadOrigin;
use tessia::domain::radicacion::{RadState, RadStatus, Step};
use tessia::domain::{Phase, PhaseState};
use tessia::nt::{MemoryNt, NtRow};
use tessia::phases::common::PhaseRequest;
use tessia::phases::convenio_preparar::{
    Args, PrepareRequest, commit_preparation, plan_preparation, run, run_phase, summary_of,
};
use tessia::stores::parametros::Parametros;
use tessia::testing::{MockResponse, MockServer};

const ID: &str = "Convenio_21102026_21102026";
const HEADER: &str = "nit|plan|subplan|factura|tipo de identificacion|no. documento|no. formula|fecha|plu|articulo|cantidad|vlr. unitario|vlr. total|cuota|mipres|cum";

fn nt_row(code: &str, cum: &str, product_type: &str, price: &str) -> NtRow {
    NtRow {
        cod_gen_siesa: Some(code.into()),
        cum_facturar: Some(cum.into()),
        tipo_producto: Some(product_type.into()),
        precio_contrato: Some(price.parse().expect("price")),
        ffm_rips: Some("TABLETA".into()),
        umm_rips: Some("500".into()),
        upr_rips: Some("MG".into()),
        ..NtRow::default()
    }
}

fn catalog() -> MemoryNt {
    MemoryNt::new().with_version(
        "2026-10",
        vec![
            ("1".into(), nt_row("20010", "19950404-1", "714", "1500")),
            ("2".into(), nt_row("30020", "12345", "799", "0")),
        ],
    )
}

/// One CSV row: invoice, subplan, document, plu, cum, unit, total, formula, mipres.
#[allow(clippy::too_many_arguments)]
fn row(
    invoice: &str,
    subplan: &str,
    document: &str,
    plu: &str,
    cum: &str,
    unit: &str,
    total: &str,
    formula: &str,
    mipres: &str,
) -> String {
    format!(
        "900000001|PLAN|{subplan}|{invoice}|CC|{document}|{formula}|2026-10-21 10:00:00|{plu}|PRODUCTO|1|{unit}|{total}|0|{mipres}|{cum}"
    )
}

fn pos(invoice: &str, document: &str, unit: &str, total: &str) -> String {
    row(
        invoice,
        "EVENTO POS",
        document,
        "20010_01",
        "19950404-1",
        unit,
        total,
        "",
        "",
    )
}

fn csv(rows: &[String]) -> String {
    format!("{HEADER}\n{}\n", rows.join("\n"))
}

fn write_csv(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write csv");
    path
}

fn setup() -> (TestDb, AppContext, tempfile::TempDir) {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db).with_nt(Arc::new(catalog()));
    (db, ctx, tempfile::tempdir().expect("tempdir"))
}

fn request(path: &Path) -> PrepareRequest {
    PrepareRequest {
        source: path.to_string_lossy().into_owned(),
        max_amount: Some("250000".into()),
        ..PrepareRequest::default()
    }
}

fn mixed_csv() -> String {
    csv(&[
        pos("EVS-1", "111", "1500,00", "1500"),
        pos("EVS-2", "222", "1600", "1600"),
        row(
            "EVS-3",
            "EVENTO POS",
            "333",
            "99999_01",
            "000",
            "1",
            "1",
            "",
            "",
        ),
        pos("EVS-4", "444", "1500", "1500"),
        pos("EVS-4", "555", "1500", "1500"),
        pos("EVS-5", "666", "1500", "300000"),
    ])
}

fn states(ctx: &AppContext) -> std::collections::HashMap<String, (PhaseState, String, i32)> {
    ctx.proceso()
        .load(Phase::Convenio, None)
        .expect("states")
        .into_iter()
        .map(|(k, v)| (k, (v.state, v.motivo, v.intentos)))
        .collect()
}

#[test]
fn a_local_csv_is_registered_prepared_and_stored() {
    let (_db, ctx, dir) = setup();
    let path = write_csv(dir.path(), &format!("{ID}.csv"), &mixed_csv());
    let plan = plan_preparation(&ctx, &request(&path), None).expect("plan");

    assert_eq!(plan.nt_version, "2026-10");
    assert_eq!(plan.nt_entries, 1, "only the catalog rows the CSV names");
    let summary = summary_of(&plan);
    assert_eq!(
        summary[0],
        format!("convenio: {ID}.csv (load {ID}, 2026-10-21..2026-10-21)")
    );
    assert_eq!(
        summary[2],
        "lines read=6 skipped=0 | invoices prepared=1 excluded=3 manual_review=1 | medicines=1 other services=0 copay lines=0"
    );
    commit_preparation(&ctx, &plan).expect("commit");

    // The cargue and its bucket artifact.
    let cargue = ctx.cargues().newest(ID).expect("newest").expect("cargue");
    assert_eq!(
        cargue.origin(),
        LoadOrigin::Medicar,
        "a Medicar-looking name"
    );
    assert_eq!(cargue.seq, plan.cargue);
    assert_eq!(
        ctx.artifacts()
            .fetch_bytes(ID, &format!("{ID}.csv"))
            .expect("artifact"),
        mixed_csv().as_bytes()
    );

    // The dataset.
    let ds = ctx.dataset().read(cargue.seq, None).expect("dataset");
    let prepared: Vec<_> = ds.invoices.iter().map(|i| i.invoice.as_str()).collect();
    assert_eq!(prepared, ["EVS1"]);
    let line = &ds.invoices[0].lines[0];
    assert_eq!(line.cum, "19950404-1");
    assert_eq!(line.product_type, "714");
    assert_eq!(line.pharmaceutical_form, "TABLETA");
    assert_eq!(line.measure_unit, "MG");
    assert_eq!(line.concentration, "500");
    assert_eq!(line.agreed_price, "1500");
    assert_eq!(line.unit_value, "1500", "the load rules drop ,00");
    assert_eq!(ds.preparation.nt_version, "2026-10");
    assert_eq!(
        ds.preparation.max_amount,
        Some("250000".parse().expect("max"))
    );
    assert_eq!(ds.preparation.stats.medicines, 1);
    assert_eq!(ds.preparation.load.id, ID);
    assert_eq!(ds.excluded.len(), 3);
    assert_eq!(ds.manual_review.len(), 1);
    assert_eq!(ds.manual_review[0].invoice, "EVS4");
    assert_eq!(
        ds.manual_review[0].text(),
        "MULTIPLES_AFILIADOS_EN_FACTURA: Factura con lineas de mas de un afiliado (2 documentos distintos); revisar en Medicar."
    );

    // The phase states.
    let states = states(&ctx);
    assert_eq!(states["EVS1"], (PhaseState::Done, String::new(), 1));
    assert_eq!(states["EVS2"].0, PhaseState::Excluded);
    assert!(
        states["EVS2"]
            .1
            .starts_with("PRECIO_MAYOR_NOTA_TECNICA: precio convenio COP=1600")
    );
    assert_eq!(
        states["EVS3"].1,
        "No cruza nota tecnica: PLU=99999_01, codigoGenerico=99999, CUM convenio=000"
    );
    assert_eq!(states["EVS4"].0, PhaseState::RevisionManual);
    assert!(
        states["EVS4"]
            .1
            .starts_with("MULTIPLES_AFILIADOS_EN_FACTURA: ")
    );
    assert_eq!(
        states["EVS5"].1,
        "MONTO_MAXIMO_SUPERADO: total COP=300000, maximo COP=250000"
    );
    let stored = ctx.proceso().load(Phase::Convenio, None).expect("states");
    assert_eq!(stored["EVS1"].cargue_id.as_deref(), Some(ID));
}

#[test]
fn preparing_again_is_idempotent_by_sha_and_bumps_the_versions() {
    let (_db, ctx, dir) = setup();
    let path = write_csv(dir.path(), "Manual_a.csv", &mixed_csv());
    let first = plan_preparation(&ctx, &request(&path), None).expect("plan");
    commit_preparation(&ctx, &first).expect("commit");
    let second = plan_preparation(&ctx, &request(&path), None).expect("plan again");
    assert_eq!(second.cargue, first.cargue, "same bytes, same cargue");
    commit_preparation(&ctx, &second).expect("commit again");
    let ds = ctx.dataset().read(first.cargue, None).expect("dataset");
    assert_eq!(ds.invoices[0].version, 1, "replaced once");
    assert_eq!(
        ctx.cargues()
            .newest("Manual_a")
            .expect("n")
            .expect("c")
            .origin(),
        LoadOrigin::Manual
    );
    assert_eq!(states(&ctx)["EVS1"].2, 1, "Go stores attempts = 1");
}

#[test]
fn an_older_upload_of_a_load_is_superseded() {
    let (_db, ctx, dir) = setup();
    let old = write_csv(dir.path(), &format!("{ID}.csv"), &mixed_csv());
    plan_preparation(&ctx, &request(&old), None).expect("old");
    let newer_dir = tempfile::tempdir().expect("dir");
    let newer = write_csv(
        newer_dir.path(),
        &format!("{ID}.csv"),
        &csv(&[pos("EVS-9", "1", "1500", "1500")]),
    );
    plan_preparation(&ctx, &request(&newer), None).expect("newer");
    let error = plan_preparation(&ctx, &request(&old), None).expect_err("superseded");
    assert!(
        format!("{error:#}").contains("ya existe un cargue más reciente"),
        "{error:#}"
    );
}

#[test]
fn an_existing_cargue_is_read_from_the_bucket_and_invoices_merge() {
    let (_db, ctx, dir) = setup();
    let path = write_csv(dir.path(), &format!("{ID}.csv"), &mixed_csv());
    let first = plan_preparation(&ctx, &request(&path), None).expect("plan");
    commit_preparation(&ctx, &first).expect("commit");

    // Re-prepare only EVS5 with a higher cap, by cargue id (no local file needed).
    let again = PrepareRequest {
        source: ID.into(),
        max_amount: Some("500000".into()),
        only: vec!["evs-5".into()],
        limit: 0,
    };
    let plan = plan_preparation(&ctx, &again, None).expect("by id");
    assert_eq!(plan.cargue, first.cargue);
    assert_eq!(plan.result.invoices.len(), 1);
    commit_preparation(&ctx, &plan).expect("commit merge");

    let ds = ctx.dataset().read(first.cargue, None).expect("dataset");
    let prepared: Vec<_> = ds.invoices.iter().map(|i| i.invoice.as_str()).collect();
    assert_eq!(
        prepared,
        ["EVS1", "EVS5"],
        "EVS1 kept, EVS5 replaced and now prepared"
    );
    assert_eq!(ds.excluded.len(), 2, "EVS5 left the excluded list");
    assert_eq!(states(&ctx)["EVS5"].0, PhaseState::Done);
    assert_eq!(states(&ctx)["EVS2"].0, PhaseState::Excluded, "untouched");
}

#[test]
fn unknown_invoices_warn_and_the_command_fails_only_when_none_matches() {
    let (_db, ctx, dir) = setup();
    let path = write_csv(dir.path(), "Manual_u.csv", &mixed_csv());
    let mut req = request(&path);
    req.only = vec!["EVS1".into(), "EVS99".into()];
    let plan = plan_preparation(&ctx, &req, None).expect("one matches");
    assert_eq!(plan.result.unknown_invoices, ["EVS99"]);
    req.only = vec!["EVS98".into(), "EVS99".into()];
    let error = plan_preparation(&ctx, &req, None).expect_err("none matches");
    assert_eq!(error.to_string(), "none of the --invoices is in the CSV");
}

#[test]
fn the_cap_comes_from_the_flag_then_the_stored_parameters_then_the_settings() {
    let (_db, ctx, dir) = setup();
    let path = write_csv(
        dir.path(),
        "Manual_c.csv",
        &csv(&[pos("EVS-5", "1", "1500", "300000")]),
    );
    let mut req = request(&path);
    req.max_amount = None;
    let error = plan_preparation(&ctx, &req, None).expect_err("no cap");
    assert_eq!(
        error.to_string(),
        "the maximum amount is required: --monto-maximo 100000 or TESSIA_CONVENIO_MAX_INVOICE_AMOUNT"
    );
    assert!(
        ctx.cargues().newest("Manual_c").expect("n").is_none(),
        "nothing registered"
    );

    let configured = ctx.clone().with_settings(Settings::from_pairs([(
        "TESSIA_CONVENIO_MAX_INVOICE_AMOUNT",
        "100000",
    )]));
    let plan = plan_preparation(&configured, &req, None).expect("configured");
    assert_eq!(
        plan.result.excluded[0].reason,
        "MONTO_MAXIMO_SUPERADO: total COP=300000, maximo COP=100000"
    );

    let mut stored = Parametros::defaults("400000".parse().expect("amount"));
    stored.automatic_subplans = vec!["EVENTO NO PBS".into()];
    ctx.parametros().set(&stored).expect("params");
    let plan = plan_preparation(&ctx, &req, None).expect("stored");
    assert_eq!(
        plan.result.excluded[0].reason,
        "Subplan 'EVENTO POS' no se radica automaticamente; use --invoices para forzarlo.",
        "the stored allowlist replaces the defaults"
    );
    req.max_amount = Some("100".into());
    req.only = vec!["EVS5".into()];
    let plan = plan_preparation(&ctx, &req, None).expect("explicit");
    assert_eq!(
        plan.max.to_string(),
        "100",
        "the flag wins over the stored cap"
    );
    assert_eq!(
        plan.automatic_subplans,
        ["EVENTO NO PBS"],
        "the stored allowlist still applies"
    );
    assert!(
        plan.result.excluded[0]
            .reason
            .starts_with("MONTO_MAXIMO_SUPERADO")
    );
}

#[test]
fn finished_invoices_never_consume_the_limit() {
    let (_db, ctx, dir) = setup();
    let rows: Vec<String> = (1..=4)
        .map(|n| pos(&format!("EVS-{n}"), "1", "1500", "1500"))
        .collect();
    let path = write_csv(dir.path(), "Manual_l.csv", &csv(&rows));
    let mut done = RadState::new("EVS1", Step::Crc);
    done.status = RadStatus::Confirmed;
    ctx.radicacion().save(&done).expect("rad");
    let mut req = request(&path);
    req.limit = 2;
    let plan = plan_preparation(&ctx, &req, None).expect("plan");
    let ids: Vec<_> = plan
        .result
        .invoices
        .iter()
        .map(|i| i.invoice.as_str())
        .collect();
    assert_eq!(
        ids,
        ["EVS1", "EVS2", "EVS3"],
        "EVS1 is finished: 2 pending were prepared besides it"
    );
    assert_eq!(plan.result.stats.finished, 1);
    assert_eq!(plan.result.stats.ignored, 1);
    let lines = summary_of(&plan);
    assert!(lines.contains(&"already finished (not counted in --limit): 1".to_owned()));
    assert!(
        lines
            .contains(&"limit: 2/2 prepared (evaluated 2, excluded 0, manual_review 0)".to_owned())
    );
    assert_eq!(plan.result.remaining, ["EVS4"]);
}

fn mipres_server(deliveries: &'static str) -> MockServer {
    MockServer::start(move |request| {
        if request.path.starts_with("/api/GenerarToken/") {
            MockResponse::json(200, "\"secondary-token\"")
        } else if request.path.starts_with("/api/EntregaXPrescripcion/") {
            MockResponse::json(200, deliveries)
        } else {
            MockResponse::text(404, "")
        }
    })
}

fn no_pbs_csv() -> String {
    csv(&[
        row(
            "EVS-1",
            "EVENTO NO PBS",
            "1",
            "20010_01",
            "19950404-1",
            "1500",
            "1500",
            "7",
            "P1",
        ),
        row(
            "EVS-2",
            "EVENTO NO PBS",
            "2",
            "20010_01",
            "19950404-1",
            "1500",
            "1500",
            "8",
            "P1",
        ),
        pos("EVS-3", "3", "1500", "1500"),
    ])
}

fn with_mipres(ctx: AppContext, server: &MockServer) -> AppContext {
    let settings = Settings::from_pairs([
        ("TESSIA_MIPRES_BASE_URL", server.url()),
        ("TESSIA_MIPRES_NIT", "900123456"),
        ("TESSIA_MIPRES_TOKEN_PPAL", "primary"),
    ]);
    ctx.with_settings(settings)
}

#[test]
fn evento_no_pbs_invoices_are_checked_against_mipres() {
    let (_db, ctx, dir) = setup();
    let server = mipres_server(r#"[{"ID":"007","EstEntrega":2},{"ID":"8","EstEntrega":0}]"#);
    let ctx = with_mipres(ctx, &server);
    let path = write_csv(dir.path(), "Manual_m.csv", &no_pbs_csv());
    let plan = plan_preparation(&ctx, &request(&path), None).expect("plan");
    commit_preparation(&ctx, &plan).expect("commit");

    let ds = ctx.dataset().read(plan.cargue, None).expect("dataset");
    let by_name = |n: &str| ds.invoice(n).map(|i| i.id_mipres.clone());
    assert_eq!(
        by_name("EVS1").as_deref(),
        Some("007"),
        "the delivery id as MIPRES spells it"
    );
    assert_eq!(
        by_name("EVS3").as_deref(),
        Some(""),
        "POS invoices are not checked"
    );
    let states = states(&ctx);
    assert_eq!(
        states["EVS2"],
        (PhaseState::Excluded, "MIPRES ANULADO".into(), 1)
    );

    let requests = server.requests();
    let paths: Vec<&str> = requests.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "/api/GenerarToken/900123456/primary",
            "/api/EntregaXPrescripcion/900123456/secondary-token/P1",
        ],
        "one token, one query per distinct prescription"
    );
}

#[test]
fn mipres_configuration_is_checked_before_anything_is_registered() {
    let (_db, ctx, dir) = setup();
    let path = write_csv(dir.path(), "Manual_n.csv", &no_pbs_csv());
    let error = plan_preparation(&ctx, &request(&path), None).expect_err("no mipres settings");
    let text = format!("{error:#}");
    assert!(
        text.contains("EVENTO NO PBS invoices need the MIPRES delivery check"),
        "{text}"
    );
    assert!(text.contains("mipres.nit"), "{text}");
    assert!(ctx.cargues().newest("Manual_n").expect("n").is_none());
    assert!(ctx.artifacts().list("Manual_n").expect("list").is_empty());
}

#[test]
fn rejected_mipres_credentials_abort_before_the_nt_is_read() {
    let (_db, ctx, dir) = setup();
    let server = MockServer::start(|_| MockResponse::json(401, "{}"));
    let ctx = with_mipres(ctx, &server).with_nt(Arc::new(MemoryNt::new()));
    let path = write_csv(dir.path(), "Manual_r.csv", &no_pbs_csv());
    let error = plan_preparation(&ctx, &request(&path), None).expect_err("401");
    let text = format!("{error:#}");
    assert!(text.contains("MIPRES check aborted"), "{text}");
    assert!(!text.contains("primary"), "{text}");
    assert!(ctx.cargues().newest("Manual_r").expect("n").is_none());
}

#[test]
fn an_empty_nt_is_an_error() {
    let (_db, ctx, dir) = setup();
    let ctx = ctx.with_nt(Arc::new(MemoryNt::new()));
    let path = write_csv(dir.path(), "Manual_e.csv", &mixed_csv());
    let error = plan_preparation(&ctx, &request(&path), None).expect_err("empty NT");
    assert!(error.to_string().contains("Nota Técnica"), "{error}");
}

#[test]
fn run_phase_prepares_an_existing_cargue_and_reports_the_outcomes() {
    let (_db, ctx, dir) = setup();
    let path = write_csv(dir.path(), &format!("{ID}.csv"), &mixed_csv());
    // Register the cargue through the command (with a cap), then let the
    // orchestrator entry point re-run it with the stored parameters.
    let mut stored = Parametros::defaults("250000".parse().expect("amount"));
    stored.automatic_subplans = vec!["EVENTO POS".into()];
    ctx.parametros().set(&stored).expect("params");
    run(
        &ctx,
        Args {
            source: Some(path.to_string_lossy().into_owned()),
            ..Args::default()
        },
    )
    .expect("run");
    let report = run_phase(
        &ctx,
        &PhaseRequest {
            cargue_id: ID.into(),
            invoices: Some(vec!["EVS1".into(), "EVS2".into()]),
            workers: None,
        },
    )
    .expect("run_phase");
    let counters = report.counters();
    assert_eq!((counters.done, counters.excluded), (1, 1));
    let ds = ctx
        .dataset()
        .read_current(ID, None)
        .expect("read")
        .expect("dataset");
    assert_eq!(ds.invoices.len(), 1);
    assert_eq!(ds.excluded.len(), 3, "the merge kept the other exclusions");
    assert_eq!(ds.manual_review.len(), 1);
}

#[test]
fn bad_arguments_are_reported() {
    let (_db, ctx, _dir) = setup();
    let negative = run(
        &ctx,
        Args {
            source: Some("x".into()),
            limit: -1,
            ..Args::default()
        },
    );
    assert_eq!(
        negative.expect_err("negative").to_string(),
        "--limit must be >= 0"
    );
    assert!(run(&ctx, Args::default()).is_err(), "a source is required");
    let both = Args {
        source: Some("a".into()),
        cargue: Some("b".into()),
        ..Args::default()
    };
    assert!(run(&ctx, both).is_err());
    let missing = plan_preparation(
        &ctx,
        &PrepareRequest {
            source: "NoExiste_1".into(),
            max_amount: Some("1".into()),
            ..PrepareRequest::default()
        },
        None,
    );
    assert!(
        missing
            .expect_err("unknown cargue")
            .to_string()
            .contains("no existe el cargue")
    );
    let nowhere = plan_preparation(
        &ctx,
        &PrepareRequest {
            source: "/no/such/file.csv".into(),
            max_amount: Some("1".into()),
            ..PrepareRequest::default()
        },
        None,
    );
    assert!(
        nowhere
            .expect_err("missing file")
            .to_string()
            .contains("no existe el archivo")
    );
}
