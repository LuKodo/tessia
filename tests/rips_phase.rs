//! `rips generar` end to end over a throwaway Postgres database and the
//! in-memory bucket: selection gates, claims, `derechos/<F>.json` artifacts,
//! published `<F>.json` and the recorded states.

use std::collections::HashMap;

use tessia::config::Settings;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::{Outcome, Phase, PhaseState};
use tessia::phases::common::PhaseRequest;
use tessia::phases::rips::input::{RipsInput, RipsLine};
use tessia::phases::rips::{Candidate, Execution, OTHER_MACHINE, RipsOptions, Selection, execute};
use tessia::stores::proceso::{ClaimItem, SaveRow};

const CARGUE: &str = "Convenio_21102026_21102026";

fn fixed_hour(_: &str, _: &str, _: usize) -> String {
    "08:15".to_owned()
}

fn opts() -> RipsOptions {
    RipsOptions {
        hour: fixed_hour,
        ..RipsOptions::new("806011811", "130018500201")
    }
}

fn line(source_line: i32, product_type: &str) -> RipsLine {
    RipsLine {
        source_line,
        doc_type: "CC".into(),
        doc_number: "900000001".into(),
        date: "2026-09-01 10:22:33".into(),
        article: "ACETAMINOFEN (500MG) TABLETA".into(),
        quantity: "2".into(),
        unit_value: "1500".into(),
        total_value: "3000".into(),
        copay: "0".into(),
        cum: "19950404-1".into(),
        diagnosis_code: "i10x".into(),
        product_type: product_type.into(),
        pharmaceutical_form: "TABLETA".into(),
        concentration: "500".into(),
        measure_unit: "168".into(),
        ..RipsLine::default()
    }
}

fn input(invoice: &str, lines: Vec<RipsLine>) -> RipsInput {
    RipsInput {
        invoice: invoice.into(),
        lines,
    }
}

fn prepared(invoice: &str) -> Candidate {
    Candidate::Prepared(input(invoice, vec![line(2, "714"), line(3, "799")]))
}

const DONE_RIGHTS: &str = r#"{"version":1,"estado":"DONE","afiliado":{"tipo_documento":"CC",
    "numero_documento":"900000001","derechos":{"tipo_usuario":"01","zona":"U",
    "municipio_dane":"13001","sexo":"F","fecha_nacimiento":"1990-01-02"},"origen":"validador"}}"#;

fn publish_rights(ctx: &AppContext, invoice: &str, json: &str) {
    ctx.artifacts()
        .publish_bytes(CARGUE, &format!("derechos/{invoice}.json"), json.as_bytes())
        .expect("publish derechos");
}

fn selection(candidates: Vec<Candidate>) -> Selection {
    Selection {
        candidates,
        unknown: vec![],
    }
}

fn run(ctx: &AppContext, req: &PhaseRequest, candidates: Vec<Candidate>) -> Execution {
    execute(ctx, req, selection(candidates), &opts()).expect("execute")
}

fn rips_states(
    ctx: &AppContext,
    invoices: &[&str],
) -> HashMap<String, tessia::stores::proceso::ProcesoState> {
    let names: Vec<String> = invoices.iter().map(|s| (*s).to_owned()).collect();
    ctx.proceso()
        .load(Phase::Rips, Some(&names))
        .expect("load states")
}

fn seed(ctx: &AppContext, invoice: &str, phase: Phase, outcome: Outcome) {
    ctx.proceso()
        .save(&SaveRow {
            invoice: invoice.into(),
            phase,
            outcome,
            intentos: 1,
            cargue_id: Some(CARGUE.into()),
        })
        .expect("seed state");
}

fn skip_codes(exec: &Execution) -> Vec<(String, String)> {
    exec.skipped
        .iter()
        .map(|s| (s.invoice.clone(), s.code.clone()))
        .collect()
}

#[test]
fn ready_invoices_publish_their_json_and_are_recorded_done() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    publish_rights(&ctx, "FVS1", DONE_RIGHTS);
    publish_rights(&ctx, "FVS2", DONE_RIGHTS);

    let exec = run(
        &ctx,
        &PhaseRequest::new(CARGUE),
        vec![prepared("FVS1"), prepared("FVS2")],
    );

    assert!(exec.errors.is_empty(), "{:?}", exec.errors);
    assert_eq!(
        exec.written,
        [
            ("FVS1".to_owned(), "FVS1.json".to_owned(), 1, 1),
            ("FVS2".to_owned(), "FVS2.json".to_owned(), 1, 1)
        ]
    );
    assert_eq!(exec.report.counters().done, 2);
    let bytes = ctx
        .artifacts()
        .fetch_bytes(CARGUE, "FVS1.json")
        .expect("artifact");
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(text.starts_with("{\n  \"numDocumentoIdObligado\": \"806011811\","));
    assert!(text.contains("\"numFactura\": \"FVS1\""));
    assert!(text.contains("\"fechaDispensAdmon\": \"2026-09-01 08:15\""));
    assert!(text.contains("\"nomTecnologiaSalud\": \"ACETAMINOFEN 500MG TABLETA\""));
    assert!(text.ends_with("\n}"), "no trailing newline");
    let states = rips_states(&ctx, &["FVS1", "FVS2"]);
    for invoice in ["FVS1", "FVS2"] {
        let state = &states[invoice];
        assert_eq!((state.state, state.motivo.as_str()), (PhaseState::Done, ""));
        assert_eq!(state.intentos, 1);
        assert_eq!(state.cargue_id.as_deref(), Some(CARGUE));
        assert_eq!(state.claimed_by, None);
    }
}

#[test]
fn a_second_run_rewrites_the_file_and_continues_the_attempts() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    publish_rights(&ctx, "FVS1", DONE_RIGHTS);
    let req = PhaseRequest::new(CARGUE);
    run(&ctx, &req, vec![prepared("FVS1")]);
    let first = ctx
        .artifacts()
        .fetch_bytes(CARGUE, "FVS1.json")
        .expect("first");

    let exec = run(&ctx, &req, vec![prepared("FVS1")]);

    assert_eq!(exec.report.counters().done, 1);
    assert_eq!(rips_states(&ctx, &["FVS1"])["FVS1"].intentos, 2);
    let second = ctx
        .artifacts()
        .fetch_bytes(CARGUE, "FVS1.json")
        .expect("second");
    assert_eq!(first, second, "same bytes on every regeneration");
}

#[test]
fn skip_codes_are_recorded_only_for_rips_outcomes() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    for invoice in ["OK1", "MISMATCH", "EMPTY", "BAD", "UPR"] {
        publish_rights(&ctx, invoice, DONE_RIGHTS);
    }
    publish_rights(
        &ctx,
        "MANUAL",
        r#"{"estado":"REVISION_MANUAL","motivo":"Validador de derechos no disponible"}"#,
    );
    let mut other = line(2, "714");
    other.doc_number = "900000009".into();
    let mut bad = line(5, "714");
    bad.concentration = "500 MG".into();
    let mut upr = line(2, "714");
    upr.concentration = "UPR".into();
    let candidates = vec![
        Candidate::ConvenioNotOk {
            invoice: "EXCL".into(),
            detail: "convenio EXCLUDED: MONTO_MAXIMO_SUPERADO".into(),
        },
        Candidate::Prepared(input("OK1", vec![line(2, "714")])),
        Candidate::Prepared(input("NORIGHTS", vec![line(2, "714")])),
        Candidate::Prepared(input("MANUAL", vec![line(2, "714")])),
        Candidate::Prepared(input("MISMATCH", vec![line(1, "714"), other])),
        Candidate::Prepared(input("EMPTY", vec![])),
        Candidate::Prepared(input("BAD", vec![bad])),
        Candidate::Prepared(input("UPR", vec![upr])),
    ];

    let exec = run(&ctx, &PhaseRequest::new(CARGUE), candidates);

    assert!(exec.errors.is_empty(), "{:?}", exec.errors);
    let expected: Vec<(String, String)> = [
        ("EXCL", "CONVENIO_NO_OK"),
        ("NORIGHTS", "DERECHOS_NO_OK"),
        ("MANUAL", "DERECHOS_NO_OK"),
        ("MISMATCH", "AFILIADO_NO_COINCIDE"),
        ("EMPTY", "SIN_SERVICIOS"),
        ("BAD", "DATOS_INVALIDOS"),
        ("UPR", "SIN_SERVICIOS"),
    ]
    .map(|(i, c)| (i.to_owned(), c.to_owned()))
    .to_vec();
    let mut got = skip_codes(&exec);
    got.sort();
    let mut want = expected;
    want.sort();
    assert_eq!(got, want);
    assert_eq!(
        exec.skipped
            .iter()
            .find(|s| s.invoice == "MANUAL")
            .map(|s| s.detail.as_str()),
        Some("derechos REVISION_MANUAL: Validador de derechos no disponible")
    );
    // Only the rips outcomes are stored.
    let states = rips_states(
        &ctx,
        &[
            "OK1", "EXCL", "NORIGHTS", "MANUAL", "MISMATCH", "EMPTY", "BAD", "UPR",
        ],
    );
    let mut stored: Vec<(&str, PhaseState)> =
        states.iter().map(|(k, v)| (k.as_str(), v.state)).collect();
    stored.sort_by_key(|(invoice, _)| *invoice);
    assert_eq!(
        stored,
        [
            ("BAD", PhaseState::RevisionManual),
            ("EMPTY", PhaseState::RevisionManual),
            ("MISMATCH", PhaseState::RevisionManual),
            ("OK1", PhaseState::Done),
            ("UPR", PhaseState::RevisionManual),
        ]
    );
    assert_eq!(
        states["BAD"].motivo,
        "DATOS_INVALIDOS: linea 5: concentracion no es un entero: \"500 MG\""
    );
    assert_eq!(
        states["MISMATCH"].motivo,
        "AFILIADO_NO_COINCIDE: la linea 2 no es del afiliado validado en derechos.json"
    );
    assert_eq!(
        states["EMPTY"].motivo,
        "SIN_SERVICIOS: la factura no tiene lineas en convenio.json"
    );
    // Claims of the unrecorded skips were released, nothing is left in progress.
    assert!(states.values().all(|s| s.claimed_by.is_none()));
    // No file for a skipped invoice.
    let files: Vec<String> = ctx
        .artifacts()
        .list(CARGUE)
        .expect("list")
        .into_iter()
        .map(|a| a.rel_path)
        .filter(|p| !p.starts_with("derechos/"))
        .collect();
    assert_eq!(files, ["OK1.json"]);
    assert_eq!(exec.report.counters().manual_review, 4);
}

#[test]
fn without_invoices_the_gates_skip_manual_review_and_unfinished_fev_valor() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    for invoice in ["OWN", "FEV", "FINE"] {
        publish_rights(&ctx, invoice, DONE_RIGHTS);
    }
    seed(
        &ctx,
        "OWN",
        Phase::Rips,
        Outcome::manual_review("SIN_SERVICIOS: x"),
    );
    seed(
        &ctx,
        "FEV",
        Phase::FevValor,
        Outcome::failed("VALOR_DIFERENTE"),
    );
    seed(&ctx, "FINE", Phase::FevValor, Outcome::done());
    let candidates = || vec![prepared("OWN"), prepared("FEV"), prepared("FINE")];

    let exec = run(&ctx, &PhaseRequest::new(CARGUE), candidates());

    assert_eq!(
        skip_codes(&exec),
        [
            ("OWN".to_owned(), "REVISION_MANUAL".to_owned()),
            ("FEV".to_owned(), "FEV_VALOR_NO_OK".to_owned())
        ]
    );
    assert_eq!(
        exec.skipped[0].detail,
        "rips REVISION_MANUAL: SIN_SERVICIOS: x (reprocess with --invoices)"
    );
    assert_eq!(
        exec.skipped[1].detail,
        "fev_valor FAILED: VALOR_DIFERENTE (reprocess with --invoices)"
    );
    assert_eq!(exec.report.skipped.len(), 2);
    assert_eq!(exec.written.len(), 1);
    assert_eq!(
        rips_states(&ctx, &["OWN"])["OWN"].state,
        PhaseState::RevisionManual
    );

    // With --invoices the gates are bypassed.
    let mut req = PhaseRequest::new(CARGUE);
    req.invoices = Some(vec!["OWN".into(), "FEV".into()]);
    let exec = run(&ctx, &req, vec![prepared("OWN"), prepared("FEV")]);
    assert!(exec.skipped.is_empty(), "{:?}", exec.skipped);
    assert_eq!(exec.report.counters().done, 2);
    assert_eq!(rips_states(&ctx, &["OWN"])["OWN"].state, PhaseState::Done);
}

#[test]
fn an_invoice_claimed_by_another_machine_is_left_alone() {
    let db = TestDb::new();
    let machine_a = AppContext::for_tests(&db).with_owner("machine-a");
    let machine_b = machine_a.clone().with_owner("machine-b");
    for invoice in ["TAKEN", "FREE"] {
        publish_rights(&machine_a, invoice, DONE_RIGHTS);
    }
    let claimed = machine_a
        .proceso()
        .claim_available(
            Phase::Rips,
            &[ClaimItem::new("TAKEN")],
            &machine_a.claim_options(CARGUE),
        )
        .expect("claim");
    assert_eq!(claimed.len(), 1);

    let exec = run(
        &machine_b,
        &PhaseRequest::new(CARGUE),
        vec![prepared("TAKEN"), prepared("FREE")],
    );

    assert_eq!(
        skip_codes(&exec),
        [("TAKEN".to_owned(), OTHER_MACHINE.to_owned())]
    );
    assert_eq!(
        exec.report.skipped,
        [("TAKEN".to_owned(), OTHER_MACHINE.to_owned())]
    );
    assert_eq!(exec.written.len(), 1);
    let states = rips_states(&machine_a, &["TAKEN", "FREE"]);
    assert_eq!(states["TAKEN"].state, PhaseState::EnProgreso);
    assert_eq!(states["TAKEN"].claimed_by.as_deref(), Some("machine-a"));
    assert_eq!(states["FREE"].state, PhaseState::Done);
    // Machine A finishes late: its claim is still current.
    assert!(
        machine_a
            .proceso()
            .save_outcome(Phase::Rips, "TAKEN", &Outcome::done(), claimed["TAKEN"])
            .expect("save")
    );
}

#[test]
fn two_machines_running_together_never_work_the_same_invoice_twice_at_once() {
    let db = TestDb::new();
    let machine_a = AppContext::for_tests(&db).with_owner("machine-a");
    let machine_b = machine_a.clone().with_owner("machine-b");
    let names: Vec<String> = (1..=12).map(|n| format!("FVS{n}")).collect();
    for name in &names {
        publish_rights(&machine_a, name, DONE_RIGHTS);
    }
    let candidates = || names.iter().map(|n| prepared(n)).collect::<Vec<_>>();
    let barrier = std::sync::Barrier::new(2);

    let run_on = |ctx: &AppContext| {
        barrier.wait();
        run(ctx, &PhaseRequest::new(CARGUE), candidates())
    };
    let (a, b) = std::thread::scope(|scope| {
        let (run_on, ctx_a, ctx_b) = (&run_on, &machine_a, &machine_b);
        let a = scope.spawn(move || run_on(ctx_a));
        let b = scope.spawn(move || run_on(ctx_b));
        (a.join().expect("a"), b.join().expect("b"))
    });

    assert!(a.errors.is_empty() && b.errors.is_empty());
    // Everything was processed, and what one machine did the other skipped as
    // owned elsewhere (or did later, after it finished: attempts say so).
    let done = a.written.len() + b.written.len();
    assert!(done >= 12, "{done} written");
    let states = rips_states(
        &machine_a,
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert_eq!(states.len(), 12);
    for state in states.values() {
        assert_eq!(state.state, PhaseState::Done);
    }
    let skipped_elsewhere = a.skipped.len() + b.skipped.len();
    assert_eq!(
        done + skipped_elsewhere,
        24,
        "each machine saw each invoice once"
    );
}

#[test]
fn a_corrupt_rights_artifact_fails_that_invoice_and_releases_its_claim() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    publish_rights(&ctx, "BROKEN", "not json");
    publish_rights(&ctx, "FINE", DONE_RIGHTS);

    let exec = run(
        &ctx,
        &PhaseRequest::new(CARGUE),
        vec![prepared("BROKEN"), prepared("FINE")],
    );

    assert_eq!(exec.errors.len(), 1);
    assert!(
        exec.errors[0].contains("derechos: decode derechos/BROKEN.json"),
        "{:?}",
        exec.errors
    );
    assert_eq!(exec.written.len(), 1);
    let states = rips_states(&ctx, &["BROKEN", "FINE"]);
    assert!(
        !states.contains_key("BROKEN"),
        "claim released, no row left"
    );
    assert_eq!(states["FINE"].state, PhaseState::Done);
}

#[test]
fn requesting_only_unknown_invoices_fails_and_a_partial_match_continues() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    publish_rights(&ctx, "FVS1", DONE_RIGHTS);
    let mut req = PhaseRequest::new(CARGUE);
    req.invoices = Some(vec!["NOPE".into()]);
    let none = Selection {
        candidates: vec![],
        unknown: vec!["NOPE".into()],
    };
    let err = execute(&ctx, &req, none, &opts()).expect_err("all unknown");
    assert_eq!(err.to_string(), "none of the --invoices is in the dataset");

    req.invoices = Some(vec!["FVS1".into(), "NOPE".into()]);
    let some = Selection {
        candidates: vec![prepared("FVS1")],
        unknown: vec!["NOPE".into()],
    };
    let exec = execute(&ctx, &req, some, &opts()).expect("partial match");
    assert_eq!(exec.written.len(), 1);
}

#[test]
fn options_come_from_the_company_settings_and_the_flag() {
    let settings = Settings::from_pairs([
        ("TESSIA_COMPANY_NIT_EMISOR", " 900123456 "),
        ("TESSIA_COMPANY_CODIGO_PRESTADOR", "1300185002"),
    ]);
    let default = RipsOptions::from_settings(&settings, None).expect("defaults");
    assert_eq!(default.nit, "900123456");
    assert_eq!(default.cod_prestador, "130018500201");

    let flag = RipsOptions::from_settings(&settings, Some(" 110010000102 ")).expect("flag");
    assert_eq!(flag.cod_prestador, "110010000102");

    let err = RipsOptions::from_settings(&settings, Some("1300185002")).expect_err("short");
    assert_eq!(
        err.to_string(),
        "--cod-prestador must be 12 digits (REPS code + sede), got \"1300185002\""
    );
    let letters = RipsOptions::from_settings(&settings, Some("13001850020a"));
    assert!(letters.is_err());
}
