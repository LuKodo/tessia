//! `run_phase` over a stored dataset (what `proceso run` calls).

#[path = "fixtures/derechos/support.rs"]
mod support;

use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::dataset::Line;
use tessia::domain::dataset::test_support::{DatasetBuilder, InvoiceBuilder, sample_line};
use tessia::domain::{Phase, PhaseState};
use tessia::phases::common::PhaseRequest;
use tessia::phases::derechos::result_file;
use tessia::phases::derechos::{DerechosError, run_phase};
use tessia::stores::dataset::test_support::seed_cargue;
use tessia::testing::{MockResponse, MockServer};

use support::{complete_bundle, server, settings, validate_requests};

const LOAD: &str = "Convenio_21102026_21102026";

fn line(invoice: &str, n: i32, doc_number: &str) -> Line {
    Line {
        doc_number: doc_number.into(),
        gender: "F".into(),
        birth_date: "31/01/1990".into(),
        ..sample_line(invoice, n, "10")
    }
}

fn store(ctx: &AppContext, prepared: bool, invoices: Vec<InvoiceBuilder>) {
    let seq = seed_cargue(&ctx.pool, LOAD, 'a');
    let mut builder = DatasetBuilder::new(seq)
        .load(LOAD)
        .prepared_format(prepared)
        .excluded("EX1", "VALOR_SUPERA_MONTO");
    for invoice in invoices {
        builder = builder.invoice(invoice);
    }
    ctx.dataset()
        .replace(&builder.build(), false)
        .expect("store dataset");
}

fn good_server() -> MockServer {
    server(3600, |_| MockResponse::json(200, &complete_bundle()))
}

fn state(ctx: &AppContext, invoice: &str) -> Option<PhaseState> {
    ctx.proceso()
        .load(Phase::Derechos, Some(&[invoice.to_owned()]))
        .expect("load")
        .remove(invoice)
        .map(|s| s.state)
}

#[test]
fn it_validates_the_pending_prepared_invoices_of_the_current_cargue() {
    let db = TestDb::new();
    let mock = good_server();
    let ctx = AppContext::for_tests(&db).with_settings(settings(&mock));
    store(
        &ctx,
        false,
        vec![
            InvoiceBuilder::new("EVS1")
                .with_line(line("EVS1", 1, "1"))
                .with_line(line("EVS1", 2, "1")),
            InvoiceBuilder::new("EVS2").with_line(line("EVS2", 1, "1")),
            InvoiceBuilder::new("EVS3").with_line(line("EVS3", 1, "3")),
        ],
    );

    let report = run_phase(&ctx, &PhaseRequest::new(LOAD)).expect("run_phase");

    assert_eq!(report.counters().done, 3);
    assert_eq!(validate_requests(&mock).len(), 2, "CC|1 and CC|3 once each");
    assert_eq!(
        state(&ctx, "EX1"),
        None,
        "excluded invoices are not touched"
    );
    let stored = result_file::read(&ctx.artifacts(), LOAD, "EVS1")
        .expect("read")
        .expect("file");
    assert_eq!(stored.estado, "DONE");
    // CSV sex "F" and birth 31/01/1990 agree with the validator: no differences.
    assert!(stored.diferencias_csv.is_empty());

    let again = run_phase(&ctx, &PhaseRequest::new(LOAD)).expect("second run");
    assert!(again.outcomes.is_empty(), "everything is DONE now");
}

#[test]
fn csv_differences_are_recorded_by_field_name() {
    let db = TestDb::new();
    let mock = good_server();
    let ctx = AppContext::for_tests(&db).with_settings(settings(&mock));
    let odd = Line {
        gender: "M".into(),
        birth_date: "01/02/1991".into(),
        ..line("EVS1", 1, "1")
    };
    store(
        &ctx,
        false,
        vec![InvoiceBuilder::new("EVS1").with_line(odd)],
    );

    run_phase(&ctx, &PhaseRequest::new(LOAD)).expect("run_phase");

    let stored = result_file::read(&ctx.artifacts(), LOAD, "EVS1")
        .expect("read")
        .expect("file");
    assert_eq!(stored.diferencias_csv, ["sexo", "fecha de nacimiento"]);
}

#[test]
fn named_invoices_revalidate_only_those() {
    let db = TestDb::new();
    let mock = good_server();
    let ctx = AppContext::for_tests(&db).with_settings(settings(&mock));
    store(
        &ctx,
        false,
        vec![
            InvoiceBuilder::new("EVS1").with_line(line("EVS1", 1, "1")),
            InvoiceBuilder::new("EVS2").with_line(line("EVS2", 1, "2")),
        ],
    );
    run_phase(&ctx, &PhaseRequest::new(LOAD)).expect("first");
    let request = PhaseRequest {
        cargue_id: LOAD.into(),
        invoices: Some(vec!["EVS2".into()]),
        workers: Some(2),
    };

    let report = run_phase(&ctx, &request).expect("named");

    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(report.outcomes[0].invoice, "EVS2");
    let rows = ctx.proceso().load(Phase::Derechos, None).expect("rows");
    assert_eq!((rows["EVS1"].intentos, rows["EVS2"].intentos), (1, 2));
}

#[test]
fn prepared_format_cargues_use_the_file_and_need_no_validator_settings() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    let prepared = Line {
        causal_exento: "ESSC07".into(),
        department: "13001".into(),
        municipality: "01".into(),
        gender: "MASCULINO".into(),
        birth_date: "5/03/2001".into(),
        ..line("EVS1", 1, "7")
    };
    store(
        &ctx,
        true,
        vec![InvoiceBuilder::new("EVS1").with_line(prepared)],
    );

    let report = run_phase(&ctx, &PhaseRequest::new(LOAD)).expect("run_phase");

    assert_eq!(report.counters().done, 1);
    let stored = result_file::read(&ctx.artifacts(), LOAD, "EVS1")
        .expect("read")
        .expect("file");
    let affiliate = stored.afiliado.expect("rights");
    assert_eq!(affiliate.origen, "archivo");
    assert_eq!(
        (
            affiliate.derechos.tipo_usuario.as_str(),
            affiliate.derechos.zona.as_str(),
            affiliate.derechos.sexo.as_str(),
            affiliate.derechos.fecha_nacimiento.as_str()
        ),
        ("01", "R", "M", "2001-03-05")
    );
}

#[test]
fn a_cargue_that_was_never_prepared_is_an_error() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    let err = run_phase(&ctx, &PhaseRequest::new(LOAD)).expect_err("no dataset");
    assert!(err.to_string().contains("no está preparado"), "{err:#}");
}

#[test]
fn an_authentication_stop_is_a_typed_error_after_saving() {
    let db = TestDb::new();
    let mock = server(3600, |q| {
        if q.doc_number == "bad" {
            MockResponse::text(401, "no")
        } else {
            MockResponse::json(200, &complete_bundle())
        }
    });
    let ctx = AppContext::for_tests(&db).with_settings(settings(&mock));
    store(
        &ctx,
        false,
        vec![
            InvoiceBuilder::new("EVS1").with_line(line("EVS1", 1, "1")),
            InvoiceBuilder::new("EVS2").with_line(line("EVS2", 1, "bad")),
        ],
    );
    let request = PhaseRequest {
        workers: Some(1),
        ..PhaseRequest::new(LOAD)
    };

    let err = run_phase(&ctx, &request).expect_err("authentication stop");

    assert!(matches!(
        err.downcast_ref::<DerechosError>(),
        Some(DerechosError::AuthenticationStopped { untouched: 1, .. })
    ));
    assert_eq!(state(&ctx, "EVS1"), Some(PhaseState::Done));
    assert_eq!(state(&ctx, "EVS2"), None);
}
