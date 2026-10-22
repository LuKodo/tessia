//! `rips generar` through `run_phase`, reading the prepared dataset from the
//! dataset store (excluded and manual-review invoices, `--invoices` subsets,
//! unknown invoices).

use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::dataset::test_support::{DatasetBuilder, InvoiceBuilder, sample_line};
use tessia::domain::{Phase, PhaseState};
use tessia::phases::common::PhaseRequest;
use tessia::phases::rips::run_phase;
use tessia::stores::dataset::test_support::seed_cargue;

const CARGUE: &str = "Convenio_21102026_21102026";

const RIGHTS: &str = r#"{"estado":"DONE","afiliado":{"tipo_documento":"CC",
    "numero_documento":"1000000001","derechos":{"tipo_usuario":"04","zona":"R",
    "municipio_dane":"13001","sexo":"M","fecha_nacimiento":"15/03/1985"}}}"#;

fn seeded(db: &TestDb) -> AppContext {
    let ctx = AppContext::for_tests(db);
    let seq = seed_cargue(&db.pool(), CARGUE, 'a');
    let mut medicine = sample_line("EVS1", 1, "1500");
    medicine.concentration = "500".into();
    let ds = DatasetBuilder::new(seq)
        .load(CARGUE)
        .invoice(InvoiceBuilder::new("EVS1").with_line(medicine))
        .invoice(InvoiceBuilder::new("EVS2").line("200"))
        .excluded("EVS3", "VALOR_SUPERA_MONTO")
        .manual_review("EVS4", "MULTIPLES_AFILIADOS_EN_FACTURA", "dos afiliados")
        .build();
    ctx.dataset().replace(&ds, false).expect("store dataset");
    for invoice in ["EVS1", "EVS2"] {
        ctx.artifacts()
            .publish_bytes(
                CARGUE,
                &format!("derechos/{invoice}.json"),
                RIGHTS.as_bytes(),
            )
            .expect("rights");
    }
    ctx
}

#[test]
fn run_phase_reads_the_dataset_and_reports_every_invoice() {
    let db = TestDb::new();
    let ctx = seeded(&db);

    let report = run_phase(&ctx, &PhaseRequest::new(CARGUE)).expect("run");

    assert_eq!(report.counters().done, 2);
    assert_eq!(
        report.skipped,
        [
            ("EVS3".to_owned(), "CONVENIO_NO_OK".to_owned()),
            ("EVS4".to_owned(), "CONVENIO_NO_OK".to_owned())
        ]
    );
    let json = ctx
        .artifacts()
        .fetch_bytes(CARGUE, "EVS1.json")
        .expect("EVS1.json");
    let text = String::from_utf8(json).expect("utf8");
    assert!(text.contains("\"numFactura\": \"EVS1\""));
    assert!(text.contains("\"fechaNacimiento\": \"1985-03-15\""));
    assert!(text.contains("\"tipoUsuario\": \"04\""));
    assert!(text.contains("\"codZonaTerritorialResidencia\": \"01\""));
    assert!(text.contains("\"concentracionMedicamento\": 500"));
    assert!(text.contains("\"vrServicio\": 1500"));
    // EVS2 has no concentration: "0".
    let evs2 = ctx
        .artifacts()
        .fetch_bytes(CARGUE, "EVS2.json")
        .expect("EVS2.json");
    assert!(
        String::from_utf8(evs2)
            .expect("utf8")
            .contains("\"concentracionMedicamento\": 0")
    );
    let names = ["EVS1".to_owned(), "EVS2".to_owned()];
    let states = ctx
        .proceso()
        .load(Phase::Rips, Some(&names))
        .expect("states");
    assert!(states.values().all(|s| s.state == PhaseState::Done));
}

#[test]
fn a_subset_reads_only_the_requested_invoices() {
    let db = TestDb::new();
    let ctx = seeded(&db);
    let mut req = PhaseRequest::new(CARGUE);
    req.invoices = Some(vec!["EVS2".into(), "EVS4".into(), "NOPE".into()]);

    let report = run_phase(&ctx, &req).expect("run");

    assert_eq!(report.counters().done, 1);
    assert_eq!(report.outcomes[0].invoice, "EVS2");
    assert_eq!(
        report.skipped,
        [("EVS4".to_owned(), "CONVENIO_NO_OK".to_owned())]
    );
    assert!(
        !ctx.artifacts()
            .is_published(CARGUE, "EVS1.json")
            .expect("index")
    );

    req.invoices = Some(vec!["NOPE".into()]);
    let err = run_phase(&ctx, &req).expect_err("nothing matches");
    assert_eq!(err.to_string(), "none of the --invoices is in the dataset");
}

#[test]
fn a_cargue_without_a_dataset_is_an_error() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    let err = run_phase(&ctx, &PhaseRequest::new("Convenio_01012026_01012026")).expect_err("none");
    assert!(
        err.to_string().contains("no tiene dataset preparado"),
        "{err}"
    );
}

#[test]
fn selection_maps_the_dataset_lines_without_changing_their_text() {
    use tessia::phases::rips::selection::{Candidate, select};
    let mut line = sample_line("EVS1", 7, "1.234,56");
    line.mipres = "MIP".into();
    line.formula_number = "F-1".into();
    let ds = DatasetBuilder::new(1)
        .invoice(InvoiceBuilder::new("EVS1").with_line(line))
        .build();
    let selection = select(ds, Some(&["evs-1".to_owned(), "zz".to_owned()]));
    assert_eq!(selection.unknown, ["ZZ"]);
    let [Candidate::Prepared(input)] = &selection.candidates[..] else {
        panic!("one prepared invoice")
    };
    assert_eq!(input.invoice, "EVS1");
    assert_eq!(input.lines[0].source_line, 7);
    assert_eq!(input.lines[0].total_value, "1.234,56");
    assert_eq!(
        (
            input.lines[0].mipres.as_str(),
            input.lines[0].formula_number.as_str()
        ),
        ("MIP", "F-1")
    );
}
