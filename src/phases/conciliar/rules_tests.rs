//! Tests of the pure rules (ports of `internal/conciliacion/conciliacion_test.go`).

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;

use super::catalog::{Catalog, Item};
use super::rules::{
    Estado, ORIGIN_ADDED, ORIGIN_MATCHED, RebuildInput, RuleError, analyze, cents, money, quantity,
    rebuild,
};
use crate::domain::dataset::{Invoice, Line};
use crate::fevxml::{InvoiceLine, invoice_lines};
use crate::util::decimal::parse_max_amount;

const FIXTURE: &[u8] =
    include_bytes!("../../../tests/fixtures/fevxml/conciliacion/attached_3lineas.xml");

fn item(
    cum: &str,
    plu: &str,
    code: &str,
    form: (&str, &str, &str),
    typ: &str,
    price: &str,
) -> Item {
    Item {
        cum: cum.into(),
        product_type: typ.into(),
        generic_code: code.into(),
        price: price.into(),
        form: form.0.into(),
        concentration: form.1.into(),
        unit: form.2.into(),
        plu: plu.into(),
        ..Item::default()
    }
}

/// A synthetic nota técnica for the fixture XML.
fn catalog(price57: &str) -> Catalog {
    Catalog::new(vec![
        item(
            "27363-1",
            "23_11_19",
            "23",
            ("COLFF001", "11", "1"),
            "714",
            "235",
        ),
        item(
            "20021574-2",
            "57_20_7",
            "57",
            ("COLFF002", "22", "2"),
            "714",
            price57,
        ),
        item("12345", "", "300", ("", "", ""), "799", "400"),
    ])
}

fn xml_lines() -> Vec<InvoiceLine> {
    invoice_lines(FIXTURE).expect("fixture lines")
}

fn invoice() -> Invoice {
    let base = Line {
        source_line: 7,
        invoice: "FVS9000001".into(),
        doc_type: "CC".into(),
        doc_number: "900001".into(),
        ssc: "7770001".into(),
        date: "2026-08-01".into(),
        plu: "23_11_19".into(),
        article: "UNO".into(),
        quantity: "5".into(),
        unit_value: "235".into(),
        total_value: "1175".into(),
        copay: "0".into(),
        agreement_cum: "27363-1".into(),
        cum: "27363-1".into(),
        product_type: "714".into(),
        diagnosis_code: "Z000".into(),
        ..Line::default()
    };
    let other = Line {
        source_line: 8,
        cum: "99999-9".into(),
        agreement_cum: "99999-9".into(),
        total_value: "500".into(),
        ..base.clone()
    };
    Invoice {
        invoice: "FVS9000001".into(),
        lines: vec![base, other],
        ..Invoice::default()
    }
}

fn cap(text: &str) -> Option<i64> {
    Some(cents(parse_max_amount(text).expect("cap")))
}

#[test]
fn states_follow_the_precedence() {
    let cases: [(&str, &str, i64, Option<i64>, Estado); 6] = [
        (
            "conciliada",
            "333.5",
            405_000,
            cap("100000"),
            Estado::Conciliada,
        ),
        (
            "sin monto configurado",
            "333.5",
            405_000,
            None,
            Estado::Conciliada,
        ),
        (
            "diferencia",
            "333.5",
            410_000,
            cap("100000"),
            Estado::Diferencia,
        ),
        ("tarifa", "300", 405_000, cap("100000"), Estado::Tarifa),
        (
            "precio 0 en nota tecnica",
            "0",
            405_000,
            cap("100000"),
            Estado::Tarifa,
        ),
        (
            "monto gana a tarifa",
            "300",
            405_000,
            cap("4000"),
            Estado::Monto,
        ),
    ];
    for (name, price57, xml_total, max, want) in cases {
        let r = analyze(&invoice(), xml_total, &xml_lines(), &catalog(price57), max).expect(name);
        assert_eq!((r.state, r.total), (want, 405_000), "{name}: {}", r.detail);
        assert!(!r.detail.is_empty(), "{name}");
    }
}

#[test]
fn state_details_are_the_go_texts() {
    let diff = analyze(&invoice(), 410_000, &xml_lines(), &catalog("333.5"), None).expect("diff");
    assert_eq!(
        diff.detail,
        "Las lineas XML suman COP=4050, pero el total del XML es COP=4100."
    );
    let monto = analyze(
        &invoice(),
        405_000,
        &xml_lines(),
        &catalog("333.5"),
        cap("4000"),
    )
    .expect("monto");
    assert_eq!(
        monto.detail,
        "Valor de factura supera lo permitido: total COP=4050, maximo COP=4000"
    );
    let tarifa = analyze(&invoice(), 405_000, &xml_lines(), &catalog("300"), None).expect("tarifa");
    assert_eq!(
        tarifa.detail,
        "El XML fue completado, pero al menos un precio supera la nota tecnica (57_20_7-20021574-2 precio XML=333.5 nota tecnica=300). Requiere nota credito y refacturacion; no se genera JSON/CUV ni se radica."
    );
    let ok = analyze(&invoice(), 405_000, &xml_lines(), &catalog("333.5"), None).expect("ok");
    assert_eq!(
        ok.detail,
        "La fecha se obtiene de Medicar y las lineas del XML completan exactamente el valor de la factura."
    );
}

#[test]
fn xml_lines_are_matched_to_convenio_lines_by_cum() {
    let r = analyze(&invoice(), 405_000, &xml_lines(), &catalog("333.5"), None).expect("analysis");
    let bases: Vec<_> = r.lines.iter().map(|l| l.base).collect();
    assert_eq!(bases, [Some(0), None, None]);
    assert_eq!(
        (r.lines[1].plu.as_str(), r.lines[1].cum.as_str()),
        ("57_20_7", "20021574-2")
    );
    assert_eq!(r.lines[2].item.product_type, "799");
    assert_eq!(r.unconfirmed, [1]);
}

#[test]
fn analysis_errors() {
    let mut lines = xml_lines();
    lines[2].identifier = "300_1_1-55555".into();
    let err = analyze(&invoice(), 405_000, &lines, &catalog("333.5"), None).expect_err("no cruza");
    assert_eq!(err, RuleError::NoCruza("300_1_1-55555".into()));
    assert_eq!(
        err.to_string(),
        "el PLU-CUM 300_1_1-55555 del XML no cruza con la nota tecnica"
    );
    let empty = Invoice {
        invoice: "X".into(),
        ..Invoice::default()
    };
    let err = analyze(&empty, 1, &xml_lines(), &catalog("1"), None).expect_err("no base");
    assert_eq!(err, RuleError::SinLineaBase);
}

#[test]
fn rebuild_follows_the_xml_and_keeps_the_audit() {
    let mut inv = invoice();
    inv.id_mipres = "123, 456".into();
    let r = analyze(&inv, 405_000, &xml_lines(), &catalog("333.5"), None).expect("analysis");
    let now = Utc
        .with_ymd_and_hms(2026, 9, 26, 12, 0, 0)
        .single()
        .expect("time");
    let input = RebuildInput {
        formula_date: "2026-08-15 10:20:30",
        ssc: "7770001",
        xml_total: "4050.00",
        previous_total: "1675",
        now,
    };
    let out = rebuild(&inv, &r, &input);
    assert_eq!(out.lines.len(), 3);
    assert_eq!(
        out.id_mipres, "123, 456",
        "MIPRES ids survive a reconciliation"
    );
    let (l0, l1, l2) = (&out.lines[0], &out.lines[1], &out.lines[2]);
    assert_eq!(
        (
            l0.source_line,
            l0.date.as_str(),
            l0.quantity.as_str(),
            l0.unit_value.as_str(),
            l0.total_value.as_str()
        ),
        (7, "2026-08-15 10:20:30", "10", "235", "2350")
    );
    assert_eq!(l0.article, "PRODUCTO SINTETICO UNO TABLETA");
    assert_eq!(
        (
            l0.pharmaceutical_form.as_str(),
            l0.concentration.as_str(),
            l0.measure_unit.as_str(),
            l0.agreed_price.as_str()
        ),
        ("COLFF001", "11", "1", "235")
    );
    let a0 = l0.conciliacion.as_ref().expect("line audit");
    assert_eq!(
        (
            a0.origin.as_str(),
            a0.xml_identifier.as_str(),
            a0.date_source.as_str(),
            a0.value_source.as_str()
        ),
        (ORIGIN_MATCHED, "23_11_19-27363-1", "MEDICAR", "XML")
    );
    // Added from the template (first line): patient data kept, values from the XML.
    assert_eq!(
        (l1.plu.as_str(), l1.cum.as_str(), l1.agreement_cum.as_str()),
        ("57_20_7", "20021574-2", "20021574-2")
    );
    assert_eq!(
        (
            l1.unit_value.as_str(),
            l1.total_value.as_str(),
            l1.quantity.as_str()
        ),
        ("333.5", "1000.5", "3")
    );
    assert_eq!(
        (l1.doc_number.as_str(), l1.ssc.as_str()),
        ("900001", "7770001")
    );
    assert_eq!(
        l1.conciliacion.as_ref().map(|a| a.origin.as_str()),
        Some(ORIGIN_ADDED)
    );
    assert_eq!(
        (
            l2.product_type.as_str(),
            l2.total_value.as_str(),
            l2.unit_value.as_str()
        ),
        ("799", "699.5", "349.75")
    );
    let audit = out.conciliacion.as_ref().expect("audit");
    assert_eq!(
        (
            audit.state.as_str(),
            audit.formula_date.as_str(),
            audit.ssc.as_str(),
            audit.xml_total.as_str()
        ),
        ("CONCILIADA", "2026-08-15 10:20:30", "7770001", "4050.00")
    );
    assert_eq!(
        (
            audit.previous_total.as_str(),
            audit.reconciled_total.as_str()
        ),
        ("1675", "4050")
    );
    assert_eq!(audit.reconciled_at, Some(now));
    assert_eq!(audit.original_lines.len(), 2);
    assert_eq!(audit.original_lines[1].cum, "99999-9");
    assert_eq!(audit.unconfirmed, [1i64]);
    assert_eq!(inv.lines[0].quantity, "5", "the input is not modified");

    // A second reconciliation keeps the first originals.
    let again = rebuild(
        &out,
        &r,
        &RebuildInput {
            formula_date: "2026-08-16",
            previous_total: "4050",
            ..input
        },
    );
    let originals = &again.conciliacion.as_ref().expect("audit").original_lines;
    assert_eq!(originals[0].quantity, "5");
}

#[test]
fn money_and_quantity_formats() {
    for (cents, want) in [
        (235_000, "2350"),
        (100_050, "1000.5"),
        (34_975, "349.75"),
        (1, "0.01"),
        (0, "0"),
        (-150, "-1.5"),
    ] {
        assert_eq!(money(cents), want);
    }
    for (text, want) in [
        ("10", "10"),
        ("10.00", "10"),
        ("2.5", "2.5"),
        ("1.123456", "1.1235"),
    ] {
        let d: Decimal = text.parse().expect("decimal");
        assert_eq!(quantity(d), want, "{text}");
    }
    assert_eq!(cents("2.675".parse().expect("decimal")), 268);
}
