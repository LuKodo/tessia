//! `fev verificar` against the in-memory bucket and a throwaway Postgres.

use rust_decimal::Decimal;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::{Phase, PhaseState};
use tessia::fevxml::build_xml_pdf_zip;
use tessia::phases::common::PhaseRequest;
use tessia::phases::fev::verificar::{ConvenioInvoice, verify};
use tessia::stores::proceso::{ClaimItem, ClaimOptions};

const CARGUE: &str = "Convenio_21102026_21102026";

fn attached(amount_tag: &str, amount: &str) -> Vec<u8> {
    format!(
        "<AttachedDocument xmlns:cac=\"c\" xmlns:cbc=\"b\"><cbc:Description><![CDATA[<Invoice><cac:LegalMonetaryTotal><cbc:{amount_tag} currencyID=\"COP\">{amount}</cbc:{amount_tag}></cac:LegalMonetaryTotal></Invoice>]]></cbc:Description></AttachedDocument>"
    )
    .into_bytes()
}

fn zip_with_xml(invoice: &str, xml: &[u8]) -> Vec<u8> {
    build_xml_pdf_zip(
        &format!("{invoice}.pdf"),
        b"%PDF",
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

fn convenio(invoice: &str, lines: &[&str]) -> ConvenioInvoice {
    ConvenioInvoice {
        invoice: invoice.to_owned(),
        lines: lines.iter().map(|l| (*l).to_owned()).collect(),
    }
}

fn state(ctx: &AppContext, invoice: &str) -> Option<(PhaseState, String, i32)> {
    ctx.proceso()
        .load(Phase::FevValor, Some(&[invoice.to_owned()]))
        .expect("load")
        .remove(invoice)
        .map(|s| (s.state, s.motivo, s.intentos))
}

fn dec(s: &str) -> Decimal {
    s.parse().expect("decimal")
}

fn seed(ctx: &AppContext) {
    publish(
        ctx,
        "XML/FEV1_XML_PDF.zip",
        &zip_with_xml("FEV1", &attached("PayableAmount", "100.00")),
    );
    publish(ctx, "FEV2.xml", &attached("TaxInclusiveAmount", "1,234.50"));
    publish(
        ctx,
        "ZIP/FEV3_XML_PDF.zip",
        &zip_with_xml("FEV3", &attached("PayableAmount", "500")),
    );
    publish(
        ctx,
        "FEV5.xml",
        b"<AttachedDocument>no total here</AttachedDocument>",
    );
}

fn invoices() -> Vec<ConvenioInvoice> {
    vec![
        convenio("FEV1", &["60", "40"]),
        convenio("FEV2", &["$ 1.234,50"]),
        convenio("FEV3", &["400"]),
        convenio("FEV4", &["10"]),
        convenio("FEV5", &["10"]),
    ]
}

#[test]
fn every_invoice_is_compared_printed_and_recorded() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    seed(&ctx);
    let before = ctx.artifacts().list(CARGUE).expect("list");

    let run = verify(&ctx, &PhaseRequest::new(CARGUE), &invoices()).expect("verify");

    assert_eq!(
        run.lines,
        [
            "  FEV1 OK XML=100.00 convenio=100",
            "  FEV2 OK XML=1234.50 convenio=1234.50",
            "  FEV3 VALOR_DIFERENTE XML=500 convenio=400",
            "  FEV4 XML_SIN_VALOR (no se pudo extraer el valor total del XML)",
            "  FEV5 XML_SIN_VALOR (no se pudo extraer el valor total del XML)",
            "fev verificar: ok=2 diferentes=1 sin_valor_xml=2",
        ]
    );
    assert_eq!((run.ok, run.different, run.no_xml), (2, 1, 2));
    assert_eq!(run.mismatches(), 3);
    for (invoice, want) in [
        ("FEV1", (PhaseState::Done, "")),
        ("FEV2", (PhaseState::Done, "")),
        (
            "FEV3",
            (
                PhaseState::RevisionManual,
                "VALOR_DIFERENTE XML=500 convenio=400",
            ),
        ),
        (
            "FEV4",
            (
                PhaseState::Failed,
                "XML_SIN_VALOR (no se pudo extraer el valor total del XML)",
            ),
        ),
        (
            "FEV5",
            (
                PhaseState::Failed,
                "XML_SIN_VALOR (no se pudo extraer el valor total del XML)",
            ),
        ),
    ] {
        assert_eq!(
            state(&ctx, invoice),
            Some((want.0, want.1.to_owned(), 1)),
            "{invoice}"
        );
    }
    assert_eq!(run.report.outcomes.len(), 5);
    assert_eq!(
        ctx.artifacts().list(CARGUE).expect("list"),
        before,
        "verificar writes no artifact"
    );
}

#[test]
fn xml_totals_are_stored_for_every_invoice_with_an_xml_file() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    seed(&ctx);

    verify(&ctx, &PhaseRequest::new(CARGUE), &invoices()).expect("verify");

    let names: Vec<String> = ["FEV1", "FEV2", "FEV3", "FEV4", "FEV5"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let values = ctx.valor_xml().stored_xml_values(&names).expect("values");
    assert_eq!(values["FEV1"].amount, Some(dec("100.00")));
    assert_eq!(values["FEV2"].amount, Some(dec("1234.50")));
    assert_eq!(values["FEV3"].amount, Some(dec("500.00")));
    assert_eq!(values["FEV5"].amount, None, "file read, no total");
    assert!(values["FEV5"].stamp.is_some());
    assert!(
        !values.contains_key("FEV4"),
        "no XML file, nothing to store"
    );
}

#[test]
fn every_run_rechecks_every_invoice_and_attempts_continue() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    seed(&ctx);
    let req = PhaseRequest::new(CARGUE);
    verify(&ctx, &req, &invoices()).expect("first");
    // The XML of FEV4 shows up and the convenio of FEV3 is corrected.
    publish(&ctx, "FEV4.xml", &attached("PayableAmount", "10"));
    let mut fixed = invoices();
    fixed[2] = convenio("FEV3", &["500"]);

    let run = verify(&ctx, &req, &fixed).expect("second");

    assert_eq!((run.ok, run.different, run.no_xml), (4, 0, 1));
    assert_eq!(
        state(&ctx, "FEV3"),
        Some((PhaseState::Done, String::new(), 2))
    );
    assert_eq!(
        state(&ctx, "FEV4"),
        Some((PhaseState::Done, String::new(), 2))
    );
    assert_eq!(state(&ctx, "FEV1").map(|s| s.2), Some(2));
}

#[test]
fn invoices_filter_checks_only_those() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    seed(&ctx);
    let mut req = PhaseRequest::new(CARGUE);
    req.invoices = Some(vec!["FEV3".into()]);

    let run = verify(&ctx, &req, &invoices()).expect("verify");

    assert_eq!(
        run.lines,
        [
            "  FEV3 VALOR_DIFERENTE XML=500 convenio=400",
            "fev verificar: ok=0 diferentes=1 sin_valor_xml=0",
        ]
    );
    assert_eq!(state(&ctx, "FEV1"), None);
}

#[test]
fn a_zip_without_xml_falls_back_to_the_loose_file() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    publish(
        &ctx,
        "XML/FEV6_XML_PDF.zip",
        &build_xml_pdf_zip("FEV6.pdf", b"%PDF", "FEV6.txt", b"x", None).expect("zip"),
    );
    publish(&ctx, "FEV6.xml", &attached("PayableAmount", "70"));

    let run = verify(
        &ctx,
        &PhaseRequest::new(CARGUE),
        &[convenio("FEV6", &["70"])],
    )
    .expect("verify");

    assert_eq!(run.lines[0], "  FEV6 OK XML=70 convenio=70");
    // The stored value follows the file the tracking reader locates: the ZIP.
    let values = ctx
        .valor_xml()
        .stored_xml_values(&["FEV6".into()])
        .expect("values");
    assert_eq!(values["FEV6"].amount, None);
}

#[test]
fn the_xml_zip_wins_over_the_loose_xml_and_xml_dir_over_zip_dir() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    publish(
        &ctx,
        "XML/FEV7_XML_PDF.zip",
        &zip_with_xml("FEV7", &attached("PayableAmount", "10")),
    );
    publish(
        &ctx,
        "ZIP/FEV7_XML_PDF.zip",
        &zip_with_xml("FEV7", &attached("PayableAmount", "20")),
    );
    publish(&ctx, "FEV7.xml", &attached("PayableAmount", "30"));

    let run = verify(
        &ctx,
        &PhaseRequest::new(CARGUE),
        &[convenio("FEV7", &["10"])],
    )
    .expect("verify");

    assert_eq!(run.lines[0], "  FEV7 OK XML=10 convenio=10");
}

#[test]
fn an_unreadable_convenio_value_is_reported_as_not_available() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    publish(&ctx, "FEV8.xml", &attached("PayableAmount", "70"));

    let run = verify(
        &ctx,
        &PhaseRequest::new(CARGUE),
        &[convenio("FEV8", &["70", "n/a"]), convenio("FEV9", &[])],
    )
    .expect("verify");

    assert_eq!(
        run.lines[0],
        "  FEV8 VALOR_DIFERENTE XML=70 convenio=NO_DISPONIBLE"
    );
    assert_eq!(
        state(&ctx, "FEV8").map(|s| s.0),
        Some(PhaseState::RevisionManual)
    );
}

#[test]
fn an_invoice_claimed_by_another_machine_is_skipped() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db).with_owner("machine-a:1:x");
    seed(&ctx);
    let other = ClaimOptions::new(
        "machine-b:2:y",
        Some(CARGUE),
        ctx.claim_options(CARGUE).stale_after,
    );
    ctx.proceso()
        .claim_available(Phase::FevValor, &[ClaimItem::new("FEV3")], &other)
        .expect("claim");

    let run = verify(&ctx, &PhaseRequest::new(CARGUE), &invoices()).expect("verify");

    assert_eq!(
        run.report.skipped,
        [("FEV3".to_owned(), "EN_PROCESO_OTRA_MAQUINA".to_owned())]
    );
    assert!(
        run.lines.contains(
            &"warning: FEV3 is already in execution for phase fev_valor; skipping it".to_owned()
        ),
        "{:?}",
        run.lines
    );
    assert_eq!((run.ok, run.different, run.no_xml), (2, 0, 2));
    assert_eq!(
        state(&ctx, "FEV3").map(|s| s.0),
        Some(PhaseState::EnProgreso)
    );
}
