//! `conciliar` end to end against Postgres (`TestDb`), an in-memory bucket and
//! NT, and the fake Medicar portal.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::dataset::test_support::{DatasetBuilder, InvoiceBuilder, sample_line};
use tessia::domain::dataset::{Dataset, Line};
use tessia::domain::{Outcome, Phase, PhaseState};
use tessia::fevxml::build_xml_pdf_zip;
use tessia::http::RetryPolicy;
use tessia::nt::{MemoryNt, NtRow};
use tessia::phases::common::{PhaseReport, PhaseRequest};
use tessia::phases::conciliar::fake_portal::{
    BASE_PATH, FakeFormula, FakePortal, IDENTIFIER, PASSWORD, USERNAME,
};
use tessia::phases::conciliar::medicar::{MedicarOptions, Outage, formula_retry_policy};
use tessia::phases::conciliar::{ConciliarOptions, lease_name, run_phase_detailed, run_request};
use tessia::stores::dataset::test_support::seed_cargue;
use tessia::stores::proceso::SaveRow;
use tessia::testing::{MockResponse, MockServer};

const LOAD: &str = "Convenio_21102026_21102026";
const XML: &str = include_str!("fixtures/fevxml/conciliacion/attached_3lineas.xml");

struct Env {
    _db: TestDb,
    ctx: AppContext,
    seq: i64,
}

fn nt_row(cum: &str, code: &str, typ: &str, price: &str, plu: &str, ffm: &str) -> (String, NtRow) {
    let mut row = NtRow {
        cum_facturar: Some(cum.into()),
        cod_gen_siesa: Some(code.into()),
        tipo_producto: Some(typ.into()),
        precio_contrato: Some(price.parse().expect("price")),
        ffm_rips: Some(ffm.into()),
        umm_rips: Some("11".into()),
        upr_rips: Some("1".into()),
        ..NtRow::default()
    };
    row.extra.insert("plu".into(), plu.into());
    (format!("id-{cum}"), row)
}

fn nt(price57: &str) -> MemoryNt {
    MemoryNt::new().with_version(
        "v1",
        vec![
            nt_row("27363-1", "23", "714", "235", "23_11_19", "COLFF001"),
            nt_row("20021574-2", "57", "714", price57, "57_20_7", "COLFF002"),
            nt_row("12345", "300", "799", "400", "", ""),
        ],
    )
}

fn line(invoice: &str, cum: &str, total: &str) -> Line {
    Line {
        invoice_raw: format!("{}-{}", &invoice[..3], &invoice[3..]),
        doc_number: "900001".into(),
        ssc: "7770001".into(),
        cum: cum.into(),
        agreement_cum: cum.into(),
        article: "ACETAMINOFEN/CODEINA 325+8 MG TABLETA".into(),
        ..sample_line(invoice, 1, total)
    }
}

fn dataset(seq: i64, max: Option<&str>, invoices: &[&str]) -> Dataset {
    let mut b = DatasetBuilder::new(seq).load(LOAD).nt_version("v1");
    if let Some(m) = max {
        b = b.max_amount(m.parse().expect("max"));
    }
    for name in invoices {
        b = b.invoice(
            InvoiceBuilder::new(name)
                .with_line(line(name, "27363-1", "1175"))
                .with_line(line(name, "99999-9", "500")),
        );
    }
    b.build()
}

fn xml_for(invoice: &str) -> String {
    XML.replace("FVS9000001", invoice)
}

fn publish_xml(ctx: &AppContext, invoice: &str, xml: &str) {
    let zip = build_xml_pdf_zip(
        &format!("{invoice}.pdf"),
        b"%PDF",
        &format!("{invoice}.xml"),
        xml.as_bytes(),
        None,
    )
    .expect("zip");
    ctx.artifacts()
        .publish_bytes(LOAD, &format!("XML/{invoice}_XML_PDF.zip"), &zip)
        .expect("publish zip");
}

fn env(price57: &str, max: Option<&str>, invoices: &[&str]) -> Env {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db).with_nt(Arc::new(nt(price57)));
    let seq = seed_cargue(&db.pool(), LOAD, 'a');
    ctx.dataset()
        .replace(&dataset(seq, max, invoices), false)
        .expect("dataset");
    for name in invoices {
        publish_xml(&ctx, name, &xml_for(name));
    }
    Env { _db: db, ctx, seq }
}

fn noop_retry() -> RetryPolicy {
    RetryPolicy {
        sleeper: Some(Arc::new(|_: Duration| {})),
        jitter: 0.0,
        ..formula_retry_policy()
    }
}

fn medicar(base: &str) -> MedicarOptions {
    let mut o = MedicarOptions::new(
        &format!("{base}{BASE_PATH}"),
        IDENTIFIER,
        USERNAME,
        PASSWORD,
    );
    o.retry = noop_retry();
    o
}

type Calls = Arc<Mutex<Vec<String>>>;

/// Stubs of the follow-up phases: store DONE and publish a CUV TXT.
fn options(base: &str, calls: &Calls) -> ConciliarOptions {
    let stub = |name: &'static str,
                phase: Phase,
                txt: bool,
                calls: Calls|
     -> tessia::phases::conciliar::run::FollowupFn {
        Arc::new(move |ctx: &AppContext, req: &PhaseRequest| {
            let invoices = req.invoices.clone().unwrap_or_default();
            calls
                .lock()
                .expect("calls")
                .push(format!("{name}:{}", invoices.join(",")));
            for inv in &invoices {
                ctx.proceso().save(&SaveRow {
                    invoice: inv.clone(),
                    phase,
                    outcome: Outcome::done(),
                    intentos: 1,
                    cargue_id: Some(req.cargue_id.clone()),
                })?;
                if txt {
                    ctx.artifacts().publish_bytes(
                        &req.cargue_id,
                        &format!("{inv}.txt"),
                        br#"{"CUV":"abc123"}"#,
                    )?;
                }
            }
            Ok(PhaseReport::default())
        })
    };
    ConciliarOptions {
        followup: true,
        outage: Arc::new(Outage::new()),
        medicar: Some(medicar(base)),
        rips: stub("rips", Phase::Rips, false, calls.clone()),
        cuv: stub("cuv", Phase::Cuv, true, calls.clone()),
    }
}

fn req(invoices: Option<&[&str]>) -> PhaseRequest {
    PhaseRequest {
        cargue_id: LOAD.into(),
        invoices: invoices.map(|l| l.iter().map(|s| (*s).to_owned()).collect()),
        workers: Some(2),
    }
}

fn state(ctx: &AppContext, phase: Phase, invoice: &str) -> Option<(PhaseState, String)> {
    ctx.proceso()
        .load(phase, Some(&[invoice.to_owned()]))
        .expect("load")
        .remove(invoice)
        .map(|s| (s.state, s.motivo))
}

fn portal() -> MockServer {
    FakePortal::new(vec![FakeFormula::sample()]).start()
}

fn count(server: &MockServer, method: &str, path: &str) -> usize {
    let full = format!("{BASE_PATH}{path}");
    server
        .requests()
        .iter()
        .filter(|r| r.method == method && r.path.split('?').next() == Some(full.as_str()))
        .count()
}

#[test]
fn a_reconcilable_invoice_is_rebuilt_recorded_cleaned_and_followed_up() {
    let e = env("333.5", Some("100000"), &["FVS9000001"]);
    let server = portal();
    let calls: Calls = Arc::default();
    for stale in [
        "FVS9000001.json",
        "FVS9000001.txt",
        "FVS9000001_ERROR.txt",
        "FVS9000001_VALOR_DIFERENTE.txt",
        "OTRA.json",
    ] {
        e.ctx
            .artifacts()
            .publish_bytes(
                LOAD,
                stale,
                if stale.ends_with(".txt") {
                    br#"{"CUV":"-"}"#
                } else {
                    b"x"
                },
            )
            .expect("stale");
    }
    let opts = options(server.url(), &calls);
    let exec = run_phase_detailed(&e.ctx, &req(None), &opts).expect("run");

    assert_eq!(
        exec.counts.summary_line(),
        "conciliar: conciliadas=1 bloqueadas=0 diferencia=0 revision_manual=0 fallidas=0 omitidas=0"
    );
    assert_eq!(exec.rebuilt, ["FVS9000001"]);
    assert!(exec.incomplete().is_none());
    assert_eq!(
        exec.lines,
        [
            "  FVS9000001 CONCILIADA XML=4050.00 convenio_anterior=1675 convenio_conciliado=4050 fecha_medicar=2026-08-15 10:20:30"
        ]
    );

    // The dataset was rewritten against the version read.
    let ds = e.ctx.dataset().read(e.seq, None).expect("read");
    let inv = &ds.invoices[0];
    assert_eq!(inv.version, 1);
    assert_eq!(inv.lines.len(), 3);
    assert_eq!(inv.lines[0].date, "2026-08-15 10:20:30");
    assert_eq!(inv.lines[0].total_value, "2350");
    assert_eq!(inv.lines[0].pharmaceutical_form, "COLFF001");
    let audit = inv.conciliacion.as_ref().expect("audit");
    assert_eq!(
        (
            audit.state.as_str(),
            audit.previous_total.as_str(),
            audit.reconciled_total.as_str()
        ),
        ("CONCILIADA", "1675", "4050")
    );
    assert_eq!(audit.original_lines.len(), 2);

    // Phases recorded.
    let conc = state(&e.ctx, Phase::Conciliacion, "FVS9000001").expect("conciliacion");
    assert_eq!(conc.0, PhaseState::Done);
    assert!(
        conc.1
            .starts_with("CONCILIADA: La fecha se obtiene de Medicar"),
        "{}",
        conc.1
    );
    assert_eq!(
        state(&e.ctx, Phase::FevValor, "FVS9000001"),
        Some((
            PhaseState::Done,
            "CONCILIADA por conciliar: XML=4050.00 convenio=4050".into()
        ))
    );

    // Stale outputs of that invoice are gone, others stay; the XML ZIP stays.
    let left: Vec<_> = e
        .ctx
        .artifacts()
        .list(LOAD)
        .expect("list")
        .into_iter()
        .map(|a| a.rel_path)
        .collect();
    assert_eq!(left, ["OTRA.json", "XML/FVS9000001_XML_PDF.zip"]);
    assert!(!left.contains(&"FVS9000001.json".to_owned()));
    assert!(!left.contains(&"FVS9000001_ERROR.txt".to_owned()));

    // Medicar was contacted once, with the first line's data.
    assert_eq!(
        (
            count(&server, "POST", "/procesar.php"),
            count(&server, "POST", "/modalAjax/VerFormula.php")
        ),
        (1, 1)
    );
    // The lease is released, so another machine can take it.
    assert!(
        e.ctx
            .leases()
            .acquire(
                &lease_name(LOAD, "FVS9000001"),
                "other",
                Duration::from_secs(5)
            )
            .expect("lease")
    );
    // run_phase_detailed never runs the follow-up, even with the option on.
    assert!(calls.lock().expect("calls").is_empty());
}

#[test]
fn the_standalone_command_follows_up_with_rips_and_cuv() {
    let e = env("333.5", Some("100000"), &["FVS9000001"]);
    let server = portal();
    let calls: Calls = Arc::default();
    run_request(
        &e.ctx,
        &req(Some(&["FVS9000001"])),
        &options(server.url(), &calls),
    )
    .expect("run");
    assert_eq!(
        *calls.lock().expect("calls"),
        ["rips:FVS9000001", "cuv:FVS9000001"]
    );

    // Under proceso run (followup off) nothing is called.
    let e = env("333.5", Some("100000"), &["FVS9000001"]);
    let calls: Calls = Arc::default();
    let mut opts = options(server.url(), &calls);
    opts.followup = false;
    run_request(&e.ctx, &req(None), &opts).expect("run");
    assert!(calls.lock().expect("calls").is_empty());
}

#[test]
fn a_followup_that_does_not_reach_done_fails_the_command() {
    let e = env("333.5", Some("100000"), &["FVS9000001"]);
    let server = portal();
    let calls: Calls = Arc::default();
    let mut opts = options(server.url(), &calls);
    opts.rips = Arc::new(|_, _| Ok(PhaseReport::default()));
    let err = run_request(&e.ctx, &req(None), &opts).expect_err("rips did not run");
    assert_eq!(
        err.to_string(),
        "conciliar follow-up RIPS: FVS9000001 did not reach DONE (status=\"\"): "
    );
}

#[test]
fn blocked_different_and_broken_invoices_are_recorded_and_exit_non_zero() {
    let e = env(
        "300",
        Some("100000"),
        &["FVS9000001", "FVS9000002", "FVS9000003", "FVS9000004"],
    );
    // 2: XML total 4100 (the tariff block still wins); 3: unknown PLU-CUM; 4: no XML at all.
    publish_xml(
        &e.ctx,
        "FVS9000002",
        &xml_for("FVS9000002")
            .replace(">4050.00</cbc:PayableAmount", ">4100.00</cbc:PayableAmount"),
    );
    publish_xml(
        &e.ctx,
        "FVS9000003",
        &xml_for("FVS9000003").replace("300_1_1-12345", "300_1_1-55555"),
    );
    e.ctx
        .artifacts()
        .delete(LOAD, "XML/FVS9000004_XML_PDF.zip")
        .expect("delete");
    // The NT price 300 of CUM 20021574-2 is below the XML price: 1 and 2 are BLOQUEADA_TARIFA.
    let calls: Calls = Arc::default();
    let server = portal();
    let opts = options(server.url(), &calls);
    let exec = run_phase_detailed(&e.ctx, &req(None), &opts).expect("run");

    assert_eq!(exec.counts.bloqueadas, 2, "{:?}", exec.lines);
    assert_eq!((exec.counts.revision_manual, exec.counts.omitidas), (1, 1));
    let tarifa = state(&e.ctx, Phase::Conciliacion, "FVS9000001").expect("state");
    assert_eq!(tarifa.0, PhaseState::Excluded);
    assert!(tarifa.1.starts_with("BLOQUEADA_TARIFA: El XML fue completado, pero al menos un precio supera la nota tecnica"), "{}", tarifa.1);
    let broken = state(&e.ctx, Phase::Conciliacion, "FVS9000003").expect("state");
    assert_eq!(
        broken,
        (
            PhaseState::RevisionManual,
            "ERROR_XML: el PLU-CUM 300_1_1-55555 del XML no cruza con la nota tecnica".into()
        )
    );
    assert!(
        state(&e.ctx, Phase::Conciliacion, "FVS9000004").is_none(),
        "XML_SIN_VALOR is not recorded"
    );
    assert!(
        state(&e.ctx, Phase::FevValor, "FVS9000001").is_none(),
        "fev_valor is untouched"
    );
    let err = exec.incomplete().expect("non-zero exit");
    assert!(
        err.to_string().starts_with("3 factura(s) sin conciliar: "),
        "{err}"
    );
    assert_eq!(
        count(&server, "POST", "/procesar.php"),
        0,
        "Medicar is only queried for CONCILIADA"
    );
    // Nothing was rewritten.
    assert!(
        e.ctx
            .dataset()
            .read(e.seq, None)
            .expect("read")
            .invoices
            .iter()
            .all(|i| i.version == 0)
    );

    // Second run without --invoices skips the EXCLUDED / REVISION_MANUAL ones...
    let again = run_phase_detailed(&e.ctx, &req(None), &opts).expect("again");
    assert_eq!(again.counts.omitidas, 4, "{:?}", again.lines);
    assert!(
        again
            .lines
            .iter()
            .any(|l| l.contains("skip FVS9000001 CONCILIACION EXCLUDED: BLOQUEADA_TARIFA")),
        "{:?}",
        again.lines
    );
    assert!(again.incomplete().is_none());
    // ...and --invoices retries them.
    let retried = run_phase_detailed(&e.ctx, &req(Some(&["FVS9000001"])), &opts).expect("retry");
    assert_eq!(retried.counts.bloqueadas, 1);
}

#[test]
fn matching_totals_are_skipped_without_recording() {
    let e = env("333.5", None, &["FVS9000001"]);
    let ds = {
        let mut ds = dataset(e.seq, None, &["FVS9000001"]);
        ds.invoices[0].lines[0].total_value = "2350".into();
        ds.invoices[0].lines[1].total_value = "1700".into();
        ds
    };
    e.ctx.dataset().replace(&ds, false).expect("replace");
    let calls: Calls = Arc::default();
    let exec = run_phase_detailed(&e.ctx, &req(None), &options("http://127.0.0.1:9", &calls))
        .expect("run");
    assert_eq!(exec.counts.omitidas, 1);
    assert_eq!(
        exec.lines,
        ["  skip FVS9000001 SIN_DIFERENCIA XML=4050.00 convenio=4050"]
    );
    assert_eq!(
        exec.report.skipped,
        [("FVS9000001".to_owned(), "SIN_DIFERENCIA".to_owned())]
    );
    assert!(state(&e.ctx, Phase::Conciliacion, "FVS9000001").is_none());
}

#[test]
fn the_cap_falls_back_to_the_setting_and_is_unchecked_without_it() {
    let e = env("333.5", None, &["FVS9000001"]);
    let calls: Calls = Arc::default();
    let server = portal();
    let capped = e.ctx.clone().with_settings(
        e.ctx
            .settings
            .clone()
            .with("convenio.max_invoice_amount", "4000"),
    );
    let exec =
        run_phase_detailed(&capped, &req(None), &options(server.url(), &calls)).expect("run");
    assert_eq!(exec.counts.bloqueadas, 1, "{:?}", exec.lines);
    assert!(exec.lines[0].contains("BLOQUEADA_MONTO"));

    // Unchecked: the same invoice is reconciled.
    let e = env("333.5", None, &["FVS9000001"]);
    let exec = run_phase_detailed(&e.ctx, &req(None), &options(server.url(), &calls)).expect("run");
    assert_eq!(exec.counts.conciliadas, 1);

    // An invalid setting is an error naming the cap.
    let bad = e.ctx.clone().with_settings(
        e.ctx
            .settings
            .clone()
            .with("convenio.max_invoice_amount", "abc"),
    );
    let err =
        run_phase_detailed(&bad, &req(None), &options(server.url(), &calls)).expect_err("bad cap");
    assert!(format!("{err:#}").contains("monto maximo"), "{err:#}");
}

#[test]
fn an_existing_cuv_goes_to_manual_review_without_querying_medicar() {
    let e = env("333.5", Some("100000"), &["FVS9000001"]);
    e.ctx
        .artifacts()
        .publish_bytes(
            LOAD,
            "FVS9000001.txt",
            br#"{"CodigoUnicoValidacion":"deadbeef"}"#,
        )
        .expect("txt");
    let server = portal();
    let calls: Calls = Arc::default();
    let exec = run_phase_detailed(&e.ctx, &req(None), &options(server.url(), &calls)).expect("run");
    assert_eq!(exec.counts.revision_manual, 1);
    let st = state(&e.ctx, Phase::Conciliacion, "FVS9000001").expect("state");
    assert_eq!(st.0, PhaseState::RevisionManual);
    assert!(
        st.1.starts_with("CUV_EXISTENTE: FVS9000001.txt ya tiene CUV"),
        "{}",
        st.1
    );
    assert!(server.requests().is_empty());
    assert!(
        e.ctx
            .artifacts()
            .is_published(LOAD, "FVS9000001.txt")
            .expect("kept")
    );
    assert_eq!(
        e.ctx.dataset().read(e.seq, None).expect("read").invoices[0].version,
        0
    );
}

#[test]
fn medicar_failures_map_to_states_and_a_dead_portal_is_asked_once() {
    // Portal down for every invoice: FAILED, one retry sequence for the run.
    let e = env("333.5", Some("100000"), &["FVS9000001", "FVS9000002"]);
    let down = MockServer::start(|_| MockResponse::text(503, "busy"));
    let calls: Calls = Arc::default();
    let exec = run_phase_detailed(&e.ctx, &req(None), &options(down.url(), &calls)).expect("run");
    assert_eq!(exec.counts.fallidas, 2);
    assert_eq!(
        down.requests().len(),
        4,
        "one retry sequence for both invoices"
    );
    for inv in ["FVS9000001", "FVS9000002"] {
        let c = state(&e.ctx, Phase::Conciliacion, inv).expect("state");
        assert_eq!(c.0, PhaseState::Failed);
        assert!(c.1.starts_with("MEDICAR no disponible: medicar: la consulta a Medicar fallo (GET /: HTTP 503); se reintentara en la proxima ejecucion"), "{}", c.1);
        assert_eq!(state(&e.ctx, Phase::FevValor, inv), Some((c.0, c.1)));
    }
    assert!(exec.incomplete().expect("exit").only_medicar_unavailable());
    let ds = e.ctx.dataset().read(e.seq, None).expect("read");
    assert!(
        ds.invoices
            .iter()
            .all(|i| i.version == 0 && i.lines.len() == 2),
        "nothing rewritten"
    );

    // A formula Medicar does not relate: manual review.
    let e = env("333.5", Some("100000"), &["FVS9000001"]);
    let other = FakePortal::new(vec![FakeFormula {
        invoice: "FVS-1".into(),
        ..FakeFormula::sample()
    }])
    .start();
    let exec = run_phase_detailed(&e.ctx, &req(None), &options(other.url(), &calls)).expect("run");
    assert_eq!(exec.counts.revision_manual, 1);
    let c = state(&e.ctx, Phase::Conciliacion, "FVS9000001").expect("state");
    assert_eq!(c.0, PhaseState::RevisionManual);
    assert!(
        c.1.starts_with(
            "MEDICAR: medicar: Medicar no relaciona la formula 7770001 con la factura FVS-9000001"
        ),
        "{}",
        c.1
    );
    assert_eq!(
        state(&e.ctx, Phase::FevValor, "FVS9000001").map(|s| s.0),
        Some(PhaseState::RevisionManual)
    );
}

#[test]
fn an_invoice_changed_meanwhile_fails_the_run_and_gives_the_claims_back() {
    let e = env("333.5", Some("100000"), &["FVS9000001"]);
    // The portal re-prepares the invoice while conciliar waits for the answer.
    let inner = FakePortal::new(vec![FakeFormula::sample()]);
    let (ctx, seq) = (e.ctx.clone(), e.seq);
    let server = MockServer::start(move |r| {
        if r.path.ends_with("/VerFormula.php") {
            let ds = dataset(seq, Some("100000"), &["FVS9000001"]);
            ctx.dataset().replace(&ds, true).expect("re-prepare");
        }
        inner.respond(r)
    });
    let calls: Calls = Arc::default();
    let err = run_phase_detailed(&e.ctx, &req(None), &options(server.url(), &calls))
        .expect_err("conflict");
    assert!(
        err.to_string()
            .starts_with("an invoice changed while conciliar worked on it; rerun the command: "),
        "{err}"
    );
    assert!(
        state(&e.ctx, Phase::Conciliacion, "FVS9000001").is_none(),
        "the claim was given back"
    );
    assert!(state(&e.ctx, Phase::FevValor, "FVS9000001").is_none());
    assert!(
        e.ctx
            .leases()
            .acquire(
                &lease_name(LOAD, "FVS9000001"),
                "other",
                Duration::from_secs(5)
            )
            .expect("lease released")
    );
    assert_eq!(
        e.ctx.dataset().read(e.seq, None).expect("read").invoices[0]
            .lines
            .len(),
        2
    );
}

#[test]
fn an_invoice_another_machine_works_on_is_skipped() {
    let e = env("333.5", Some("100000"), &["FVS9000001"]);
    assert!(
        e.ctx
            .leases()
            .acquire(
                &lease_name(LOAD, "FVS9000001"),
                "otra-maquina",
                Duration::from_secs(60)
            )
            .expect("lease")
    );
    let server = portal();
    let calls: Calls = Arc::default();
    let exec = run_phase_detailed(&e.ctx, &req(None), &options(server.url(), &calls)).expect("run");
    assert_eq!(
        exec.report.skipped,
        [(
            "FVS9000001".to_owned(),
            "EN_PROCESO_OTRA_MAQUINA".to_owned()
        )]
    );
    assert_eq!(exec.counts.omitidas, 1);
    assert!(server.requests().is_empty());

    // A claim held by another machine skips it too.
    let e = env("333.5", Some("100000"), &["FVS9000001"]);
    let held = tessia::stores::proceso::ClaimItem::new("FVS9000001");
    let theirs =
        tessia::stores::proceso::ClaimOptions::new("otra", Some(LOAD), Duration::from_secs(1800));
    e.ctx
        .proceso()
        .claim_available(Phase::FevValor, &[held], &theirs)
        .expect("claim");
    let exec = run_phase_detailed(&e.ctx, &req(None), &options(server.url(), &calls)).expect("run");
    assert_eq!(exec.report.skipped.len(), 1);
    assert!(server.requests().is_empty());
    assert!(
        state(&e.ctx, Phase::Conciliacion, "FVS9000001").is_none(),
        "no half claim is left"
    );
}

#[test]
fn setup_errors_are_clear() {
    // No dataset.
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db).with_nt(Arc::new(nt("1")));
    let err = run_phase_detailed(&ctx, &req(None), &ConciliarOptions::standalone())
        .expect_err("no dataset");
    assert_eq!(
        err.to_string(),
        format!("el cargue {LOAD} no tiene dataset preparado; ejecute convenio preparar")
    );

    // Medicar not configured: the keys are named and nothing is claimed.
    let e = env("333.5", Some("100000"), &["FVS9000001"]);
    let err = run_phase_detailed(&e.ctx, &req(None), &ConciliarOptions::standalone())
        .expect_err("no config");
    let text = err.to_string();
    assert!(
        text.contains("medicar.base_url") && text.contains("TESSIA_MEDICAR_PASSWORD"),
        "{text}"
    );
    assert!(state(&e.ctx, Phase::FevValor, "FVS9000001").is_none());

    // NT version no longer in Redis.
    let gone = e.ctx.clone().with_nt(Arc::new(MemoryNt::new()));
    let err =
        run_phase_detailed(&gone, &req(None), &ConciliarOptions::standalone()).expect_err("no nt");
    assert_eq!(
        err.to_string(),
        "la versión v1 de la nota técnica del cargue no está en Redis"
    );
}
