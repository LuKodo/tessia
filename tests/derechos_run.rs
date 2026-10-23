//! `derechos validar` end to end (everything but reading the dataset):
//! selection, claims, validator or prepared-format rights, cache, result
//! files, `proceso_estado` rows and the failures reported after saving.

#[path = "fixtures/derechos/support.rs"]
mod support;

use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::{Outcome, Phase, PhaseState};
use tessia::phases::derechos::result_file::{self, StoredResult};
use tessia::phases::derechos::service::InvoiceInput;
use tessia::phases::derechos::{
    DatasetInput, DerechosError, Execution, LineInput, RunOptions, Stats, execute,
};
use tessia::stores::parametros::Parametros;
use tessia::stores::proceso::{ClaimItem, ProcesoState, SaveRow};
use tessia::testing::{MockResponse, MockServer};

use support::{
    bundle, complete_bundle, invoice, server, settings, token_requests, validate_requests,
};

const CARGUE: &str = "Convenio_21102026_21102026";

fn dataset(invoices: Vec<InvoiceInput>) -> DatasetInput {
    DatasetInput {
        invoices,
        prepared_format: false,
    }
}

fn run(ctx: &AppContext, invoices: Vec<InvoiceInput>, only: &[&str]) -> Execution {
    let only: Vec<String> = only.iter().map(|s| (*s).to_owned()).collect();
    execute(
        ctx,
        CARGUE,
        dataset(invoices),
        &only,
        &RunOptions::default(),
    )
    .expect("execute")
}

fn run_error(ctx: &AppContext, invoices: Vec<InvoiceInput>, options: &RunOptions) -> anyhow::Error {
    execute(ctx, CARGUE, dataset(invoices), &[], options).expect_err("must fail")
}

fn state_of(ctx: &AppContext, invoice: &str) -> Option<ProcesoState> {
    ctx.proceso()
        .load(Phase::Derechos, Some(&[invoice.to_owned()]))
        .expect("load")
        .remove(invoice)
}

fn stored(ctx: &AppContext, invoice: &str) -> StoredResult {
    result_file::read(&ctx.artifacts(), CARGUE, invoice)
        .expect("read")
        .unwrap_or_else(|| panic!("no result file for {invoice}"))
}

fn context(db: &TestDb, mock: &MockServer) -> AppContext {
    AppContext::for_tests(db).with_settings(settings(mock))
}

fn good_server() -> MockServer {
    server(3600, |q| {
        match (q.doc_type.as_str(), q.doc_number.as_str()) {
            ("TI", "2") => MockResponse::text(400, "use CC"),
            (_, "2") => MockResponse::json(
                200,
                &bundle(
                    "CONTRIBUTIVO",
                    Some("Rural"),
                    "47001|SANTA MARTA",
                    "male",
                    "2010-05-06",
                ),
            ),
            _ => MockResponse::json(200, &complete_bundle()),
        }
    })
}

#[test]
fn it_queries_each_affiliate_once_and_stores_files_states_and_cache() {
    let db = TestDb::new();
    let mock = good_server();
    let ctx = context(&db, &mock);

    let execution = run(
        &ctx,
        vec![
            invoice("F1", "CC", "1"),
            invoice("F2", "cc", " 1 "),
            invoice("F3", "TI", "2"),
        ],
        &[],
    );

    let docs: Vec<_> = validate_requests(&mock)
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    assert_eq!(
        docs.len(),
        3,
        "CC|1 once, TI|2 and its CC fallback: {docs:?}"
    );
    assert_eq!(execution.report.counters().done, 3);
    assert_eq!(
        execution.stats,
        Stats {
            affiliates: 2,
            cached: 0,
            queried: 2,
            unavailable: 0
        }
    );
    assert!(execution.failure.is_none());
    for name in ["F1", "F2", "F3"] {
        let state = state_of(&ctx, name).expect("row");
        assert_eq!(
            (state.state, state.intentos),
            (PhaseState::Done, 1),
            "{name}"
        );
        assert_eq!(state.cargue_id.as_deref(), Some(CARGUE));
        assert!(state.claimed_by.is_none());
    }
    let f1 = stored(&ctx, "F1");
    assert_eq!(f1.estado, "DONE");
    let affiliate = f1.afiliado.expect("affiliate");
    assert_eq!(
        (
            affiliate.tipo_documento.as_str(),
            affiliate.numero_documento.as_str(),
            affiliate.origen.as_str()
        ),
        ("CC", "1", "validador")
    );
    assert_eq!(affiliate.derechos.tipo_usuario, "04");
    assert_eq!(affiliate.tipo_documento_consultado, None);
    let f3 = stored(&ctx, "F3").afiliado.expect("affiliate");
    assert_eq!(
        (
            f3.tipo_documento.as_str(),
            f3.derechos.tipo_usuario.as_str()
        ),
        ("TI", "01")
    );
    assert_eq!(f3.tipo_documento_consultado.as_deref(), Some("CC"));
    let artifacts = ctx.artifacts().list(CARGUE).expect("list");
    let paths: Vec<_> = artifacts.iter().map(|a| a.rel_path.as_str()).collect();
    assert_eq!(paths.len(), 3);
    assert!(paths.contains(&"derechos/F1.json") && paths.contains(&"derechos/F3.json"));

    // Only complete answers are cached, under the original document type.
    let long_ago = Utc::now() - ChronoDuration::hours(1);
    assert!(
        ctx.derechos_cache()
            .get("CC", "1", long_ago)
            .expect("cache")
            .is_some()
    );
    assert!(
        ctx.derechos_cache()
            .get("TI", "2", long_ago)
            .expect("cache")
            .is_some()
    );
}

#[test]
fn a_cached_affiliate_is_not_queried_and_ttl_zero_skips_the_cache() {
    let db = TestDb::new();
    let mock = good_server();
    let ctx = context(&db, &mock);
    run(&ctx, vec![invoice("F1", "CC", "1")], &[]);
    let before = validate_requests(&mock).len();

    let cached = run(&ctx, vec![invoice("F4", "CC", "1")], &[]);
    assert_eq!(
        validate_requests(&mock).len(),
        before,
        "served from the cache"
    );
    assert_eq!(
        cached.stats,
        Stats {
            affiliates: 1,
            cached: 1,
            queried: 0,
            unavailable: 0
        }
    );
    assert_eq!(stored(&ctx, "F4").afiliado.expect("rights").origen, "cache");

    let options = RunOptions {
        cache_ttl: Duration::ZERO,
        ..RunOptions::default()
    };
    let only: Vec<String> = Vec::new();
    let fresh = execute(
        &ctx,
        CARGUE,
        dataset(vec![invoice("F5", "CC", "1")]),
        &only,
        &options,
    )
    .expect("execute");
    assert_eq!(
        validate_requests(&mock).len(),
        before + 1,
        "ttl 0 does not read the cache"
    );
    assert_eq!(fresh.stats.queried, 1);
    assert_eq!(
        stored(&ctx, "F5").afiliado.expect("rights").origen,
        "validador"
    );
}

#[test]
fn a_stale_cache_entry_is_ignored() {
    let db = TestDb::new();
    let mock = good_server();
    let ctx = context(&db, &mock);
    let old = tessia::stores::derechos::Rights {
        user_type: "01".into(),
        zone: "R".into(),
        dane: "99999".into(),
        sex: "M".into(),
        birth_date: "1970-01-01".into(),
    };
    ctx.derechos_cache()
        .put("CC", "1", &old, Utc::now() - ChronoDuration::hours(30))
        .expect("seed");

    run(&ctx, vec![invoice("F1", "CC", "1")], &[]);

    assert_eq!(
        validate_requests(&mock).len(),
        1,
        "older than 24 h: asked again"
    );
    assert_eq!(
        stored(&ctx, "F1")
            .afiliado
            .expect("rights")
            .derechos
            .municipio_dane,
        "13001"
    );
}

#[test]
fn every_failure_kind_has_its_exact_reason_in_the_row_and_the_file() {
    let db = TestDb::new();
    let mock = server(3600, |q| match q.doc_number.as_str() {
        "nf" => MockResponse::json(200, r#"{"entry":[]}"#),
        "bad" => MockResponse::text(200, "<html>"),
        "inc" => MockResponse::json(200, &bundle("CONTRIBUTIVO", None, "13001|X", "male", "")),
        _ => MockResponse::json(200, &complete_bundle()),
    });
    let ctx = context(&db, &mock);

    let execution = run(
        &ctx,
        vec![
            invoice("N1", "CC", "nf"),
            invoice("N2", "CC", "bad"),
            invoice("N3", "CC", "inc"),
            invoice("N4", "CC", "   "),
            invoice("N5", "CC", "ok"),
        ],
        &[],
    );

    let expected = [
        (
            "N1",
            "Afiliado no encontrado en validador de derechos Mutual Ser",
        ),
        ("N2", "Validador de derechos no disponible"),
        (
            "N3",
            "Datos de derechos incompletos: faltan zona, fecha de nacimiento",
        ),
        ("N4", "Factura sin tipo o numero de documento del afiliado"),
    ];
    for (name, reason) in expected {
        let row = state_of(&ctx, name).expect("row");
        assert_eq!(
            (row.state, row.motivo.as_str()),
            (PhaseState::RevisionManual, reason),
            "{name}"
        );
        let file = stored(&ctx, name);
        assert_eq!(
            (file.estado.as_str(), file.motivo.as_str()),
            ("REVISION_MANUAL", reason)
        );
        assert!(file.afiliado.is_none());
    }
    assert_eq!(state_of(&ctx, "N5").expect("row").state, PhaseState::Done);
    assert_eq!(
        execution.stats,
        Stats {
            affiliates: 4,
            cached: 0,
            queried: 4,
            unavailable: 1
        }
    );
    assert!(
        execution.failure.is_none(),
        "1 of 4 is under the 50% threshold"
    );
    assert_eq!(validate_requests(&mock).len(), 4, "no document, no query");
}

#[test]
fn two_affiliates_in_one_invoice_fail_before_any_query_naming_only_the_invoice() {
    let db = TestDb::new();
    let mock = good_server();
    let ctx = context(&db, &mock);
    let mut two = invoice("F9", "CC", "111");
    two.lines.push(LineInput {
        doc_type: "CC".into(),
        doc_number: "222".into(),
        ..LineInput::default()
    });

    let err = run_error(
        &ctx,
        vec![invoice("F1", "CC", "1"), two],
        &RunOptions::default(),
    );

    let typed = err.downcast_ref::<DerechosError>().expect("typed error");
    assert_eq!(
        typed,
        &DerechosError::MultipleAffiliates {
            invoice: "F9".into()
        }
    );
    let text = format!("{err:#}");
    assert!(text.contains("F9") && !text.contains("111") && !text.contains("222"));
    assert!(mock.requests().is_empty(), "not even a login");
    assert!(state_of(&ctx, "F1").is_none(), "nothing claimed");
    assert!(ctx.artifacts().list(CARGUE).expect("list").is_empty());
}

#[test]
fn the_validator_down_alert_fails_after_saving_unless_the_stored_policy_says_otherwise() {
    let db = TestDb::new();
    let mock = server(3600, |_| MockResponse::text(200, "<html>"));
    let ctx = context(&db, &mock);
    let invoices = || {
        (1..=4)
            .map(|n| invoice(&format!("A{n}"), "CC", &n.to_string()))
            .collect::<Vec<_>>()
    };

    let execution = run(&ctx, invoices(), &[]);

    assert_eq!(
        execution.failure,
        Some(DerechosError::ValidatorUnavailable {
            unavailable: 4,
            queried: 4,
            percent: 50
        })
    );
    assert!(
        execution.failure.as_ref().expect("failure").to_string().starts_with(
            "Validador de derechos no disponible: 4 de 4 afiliados consultados fallaron tecnicamente (100%)"
        )
    );
    for n in 1..=4 {
        let row = state_of(&ctx, &format!("A{n}")).expect("saved before failing");
        assert_eq!(row.state, PhaseState::RevisionManual);
        assert_eq!(row.motivo, "Validador de derechos no disponible");
    }
    assert!(
        ctx.derechos_cache()
            .get("CC", "1", Utc::now() - ChronoDuration::hours(1))
            .expect("cache")
            .is_none()
    );

    // A stored override disables the alert (percent 0); the invoices were
    // REVISION_MANUAL, so name them to retry.
    let mut params = Parametros::defaults(rust_decimal::Decimal::from(1_000_000));
    params.derechos_unavailable_percent = 0;
    ctx.parametros().set(&params).expect("params");
    let again = run(&ctx, invoices(), &["A1", "A2", "A3", "A4"]);
    assert!(again.failure.is_none());
    assert_eq!(again.stats.unavailable, 4);
    assert_eq!(
        state_of(&ctx, "A1").expect("row").intentos,
        2,
        "attempts grow every run"
    );

    // The minimum sample also silences small runs.
    let mut params = Parametros::defaults(rust_decimal::Decimal::from(1_000_000));
    params.derechos_min_sample = 5;
    ctx.parametros().set(&params).expect("params");
    let small = run(&ctx, invoices(), &["A1", "A2", "A3", "A4"]);
    assert!(small.failure.is_none());
}

#[test]
fn an_authentication_stop_saves_what_was_resolved_and_restores_the_rest() {
    let db = TestDb::new();
    let mock = server(3600, |q| {
        if q.doc_number == "bad" {
            MockResponse::text(401, "rejected")
        } else {
            MockResponse::json(200, &complete_bundle())
        }
    });
    let ctx = context(&db, &mock);
    // C0 failed before: it must come back to FAILED / 2 attempts, not stay EN_PROGRESO.
    ctx.proceso()
        .save(&SaveRow {
            invoice: "C3".into(),
            phase: Phase::Derechos,
            outcome: Outcome::failed("ANTERIOR"),
            intentos: 2,
            cargue_id: Some(CARGUE.into()),
        })
        .expect("seed");
    let options = RunOptions {
        concurrency: 1,
        ..RunOptions::default()
    };
    let only: Vec<String> = Vec::new();

    let execution = execute(
        &ctx,
        CARGUE,
        dataset(vec![
            invoice("A1", "CC", "1"),
            invoice("B2", "CC", "bad"),
            invoice("C3", "CC", "3"),
        ]),
        &only,
        &options,
    )
    .expect("the stop is reported after saving");

    match execution.failure.as_ref().expect("authentication stop") {
        DerechosError::AuthenticationStopped { untouched, .. } => assert_eq!(*untouched, 2),
        other => panic!("unexpected failure {other:?}"),
    }
    assert!(execution.failure.as_ref().expect("failure").to_string().starts_with(
        "validador de derechos: credenciales o token rechazados aun despues de renovarlo; 2 factura(s) quedaron sin tocar, corrige las credenciales y ejecuta de nuevo"
    ));
    assert_eq!(execution.report.counters().done, 1);
    assert_eq!(state_of(&ctx, "A1").expect("row").state, PhaseState::Done);
    assert!(
        state_of(&ctx, "B2").is_none(),
        "never recorded as 'no disponible'; the claim row is gone"
    );
    let restored = state_of(&ctx, "C3").expect("restored");
    assert_eq!(
        (restored.state, restored.motivo.as_str(), restored.intentos),
        (PhaseState::Failed, "ANTERIOR", 2)
    );
    assert!(restored.claimed_by.is_none());
    let published: Vec<_> = ctx
        .artifacts()
        .list(CARGUE)
        .expect("list")
        .into_iter()
        .map(|a| a.rel_path)
        .collect();
    assert_eq!(published, ["derechos/A1.json"]);
    assert_eq!(token_requests(&mock), 2, "login plus one renewal, no more");
}

#[test]
fn invoices_claimed_by_another_machine_are_skipped() {
    let db = TestDb::new();
    let mock = good_server();
    let ctx = context(&db, &mock);
    let other = ctx.clone().with_owner("otra-maquina:1:x");
    let claimed = other
        .proceso()
        .claim_available(
            Phase::Derechos,
            &[ClaimItem::new("A1")],
            &other.claim_options(CARGUE),
        )
        .expect("other machine claims");
    assert_eq!(claimed.len(), 1);

    let execution = run(
        &ctx,
        vec![invoice("A1", "CC", "1"), invoice("B2", "CC", "1")],
        &[],
    );

    assert_eq!(
        execution.report.skipped,
        [("A1".to_owned(), "EN_PROCESO_OTRA_MAQUINA".to_owned())]
    );
    assert_eq!(execution.report.counters().done, 1);
    assert_eq!(
        state_of(&ctx, "A1").expect("still theirs").state,
        PhaseState::EnProgreso
    );
    assert_eq!(state_of(&ctx, "B2").expect("row").state, PhaseState::Done);
    assert!(
        result_file::read(&ctx.artifacts(), CARGUE, "A1")
            .expect("read")
            .is_none()
    );
}

#[test]
fn done_and_manual_review_are_skipped_unless_named() {
    let db = TestDb::new();
    let mock = server(3600, |q| match q.doc_number.as_str() {
        "nf" => MockResponse::json(200, r#"{"entry":[]}"#),
        _ => MockResponse::json(200, &complete_bundle()),
    });
    let ctx = context(&db, &mock);
    let all = || {
        vec![
            invoice("D1", "CC", "1"),
            invoice("M2", "CC", "nf"),
            invoice("P3", "CC", "3"),
        ]
    };
    run(
        &ctx,
        vec![invoice("D1", "CC", "1"), invoice("M2", "CC", "nf")],
        &[],
    );
    let before = validate_requests(&mock).len();

    let second = run(&ctx, all(), &[]);
    assert_eq!(second.selected, 1, "only the pending invoice");
    assert_eq!(second.report.outcomes.len(), 1);
    assert_eq!(second.report.outcomes[0].invoice, "P3");
    assert_eq!(state_of(&ctx, "D1").expect("row").intentos, 1);

    let named = run(&ctx, all(), &["d-1", "M2"]);
    assert_eq!(
        named.report.outcomes.len(),
        2,
        "naming re-validates DONE and REVISION_MANUAL"
    );
    assert_eq!(state_of(&ctx, "D1").expect("row").intentos, 2);
    assert_eq!(state_of(&ctx, "M2").expect("row").intentos, 2);
    assert!(validate_requests(&mock).len() > before);
}

#[test]
fn naming_only_unknown_invoices_is_an_error_but_a_partial_miss_is_not() {
    let db = TestDb::new();
    let mock = good_server();
    let ctx = context(&db, &mock);
    let only = ["ZZ9".to_owned()];
    let err = execute(
        &ctx,
        CARGUE,
        dataset(vec![invoice("F1", "CC", "1")]),
        &only,
        &RunOptions::default(),
    )
    .expect_err("all unknown");
    assert_eq!(err.to_string(), "none of the --invoices is in the dataset");
    assert!(mock.requests().is_empty());

    let ok = run(&ctx, vec![invoice("F1", "CC", "1")], &["F1", "ZZ9"]);
    assert_eq!(ok.report.counters().done, 1);
}

#[test]
fn nothing_pending_is_a_quiet_success() {
    let db = TestDb::new();
    let mock = good_server();
    let ctx = context(&db, &mock);
    let execution = run(&ctx, Vec::new(), &[]);
    assert_eq!(execution.selected, 0);
    assert!(execution.report.outcomes.is_empty() && execution.failure.is_none());
    assert!(mock.requests().is_empty());
}

#[test]
fn the_validator_path_needs_the_derechos_settings_and_names_them() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    let err = run_error(&ctx, vec![invoice("F1", "CC", "1")], &RunOptions::default());
    let text = format!("{err:#}");
    assert!(
        text.contains("derechos.token_url") && text.contains("TESSIA_DERECHOS_PASSWORD"),
        "{text}"
    );
    assert!(state_of(&ctx, "F1").is_none(), "nothing was claimed");
}

#[test]
fn consecutive_runs_share_one_login() {
    let db = TestDb::new();
    let mock = good_server();
    let ctx = context(&db, &mock);
    run(&ctx, vec![invoice("F1", "CC", "1")], &[]);
    run(&ctx, vec![invoice("F2", "CC", "5")], &[]);
    run(&ctx, vec![invoice("F3", "CC", "6")], &[]);
    assert_eq!(token_requests(&mock), 1);
}

fn prepared_line(
    doc: &str,
    regimen: &str,
    dane: &str,
    zona: &str,
    sexo: &str,
    fecha: &str,
) -> LineInput {
    LineInput {
        doc_type: "CC".into(),
        doc_number: doc.into(),
        csv_gender: sexo.into(),
        csv_birth_date: fecha.into(),
        causal_exento: regimen.into(),
        departamento: dane.into(),
        municipio: zona.into(),
    }
}

#[test]
fn prepared_format_loads_take_rights_from_the_file_without_any_validator() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db); // no derechos.* settings at all
    let invoices = vec![
        InvoiceInput {
            invoice: "P1".into(),
            lines: vec![
                prepared_line("1", "ESS207", "13001", "02", "FEMENINO", "05/03/2001"),
                prepared_line("1", "", "", "", "", ""),
            ],
        },
        InvoiceInput {
            invoice: "P2".into(),
            lines: vec![
                prepared_line("2", "EPS048", "13001", "U", "M", "2001-03-05"),
                prepared_line("2", "EPSS48", "13001", "U", "M", "2001-03-05"),
            ],
        },
        InvoiceInput {
            invoice: "P3".into(),
            lines: vec![prepared_line("3", "XX", "13001", "U", "M", "2001-03-05")],
        },
        InvoiceInput {
            invoice: "P4".into(),
            lines: vec![prepared_line("", "EPS048", "13001", "U", "M", "2001-03-05")],
        },
    ];
    let options = RunOptions::default();
    let execution = execute(
        &ctx,
        CARGUE,
        DatasetInput {
            invoices,
            prepared_format: true,
        },
        &[],
        &options,
    )
    .expect("prepared run");

    assert!(execution.failure.is_none());
    assert_eq!(execution.stats.queried, 0);
    let p1 = stored(&ctx, "P1");
    assert_eq!(p1.estado, "DONE");
    let affiliate = p1.afiliado.expect("rights");
    assert_eq!(affiliate.origen, "archivo");
    assert_eq!(
        (
            affiliate.derechos.tipo_usuario.as_str(),
            affiliate.derechos.zona.as_str(),
            affiliate.derechos.municipio_dane.as_str(),
            affiliate.derechos.sexo.as_str(),
            affiliate.derechos.fecha_nacimiento.as_str()
        ),
        ("04", "U", "13001", "F", "2001-03-05")
    );
    assert_eq!(
        state_of(&ctx, "P2").expect("row").motivo,
        "Datos de derechos inconsistentes entre las lineas de la factura (archivo): regimen"
    );
    assert_eq!(
        state_of(&ctx, "P3").expect("row").motivo,
        "Datos de derechos incompletos: faltan regimen"
    );
    assert_eq!(
        state_of(&ctx, "P4").expect("row").motivo,
        "Factura sin tipo o numero de documento del afiliado"
    );
    assert_eq!(state_of(&ctx, "P1").expect("row").state, PhaseState::Done);
    assert!(
        ctx.derechos_cache()
            .get("CC", "1", Utc::now() - ChronoDuration::hours(1))
            .expect("cache")
            .is_none(),
        "the cache is neither read nor written"
    );
}
