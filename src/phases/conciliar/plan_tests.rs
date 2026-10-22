//! Tests of the per-invoice decisions and the exit rules.

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;

use super::catalog::{Catalog, Item};
use super::medicar::MedicarError;
use super::plan::{
    Candidate, CountKey, Counts, Decision, Detail, Incomplete, OwnState, Pending, Rules,
    cuv_existente, decide, finish_pending, one_line,
};
use super::portal_html::Formula;
use crate::domain::PhaseState;
use crate::domain::dataset::{Invoice, Line};

const FIXTURE: &str =
    include_str!("../../../tests/fixtures/fevxml/conciliacion/attached_3lineas.xml");

fn item(cum: &str, plu: &str, code: &str, typ: &str, price: &str) -> Item {
    Item {
        cum: cum.into(),
        product_type: typ.into(),
        generic_code: code.into(),
        price: price.into(),
        plu: plu.into(),
        form: "F".into(),
        ..Item::default()
    }
}

fn catalog() -> Catalog {
    Catalog::new(vec![
        item("27363-1", "23_11_19", "23", "714", "235"),
        item("20021574-2", "57_20_7", "57", "714", "333.5"),
        item("12345", "", "300", "799", "400"),
    ])
}

fn invoice(totals: [&str; 2]) -> Invoice {
    let base = Line {
        invoice: "FVS9000001".into(),
        invoice_raw: "FVS-9000001".into(),
        doc_number: "900001".into(),
        ssc: "7770001".into(),
        cum: "27363-1".into(),
        total_value: totals[0].into(),
        ..Line::default()
    };
    let other = Line {
        cum: "99999-9".into(),
        total_value: totals[1].into(),
        ..base.clone()
    };
    Invoice {
        invoice: "FVS9000001".into(),
        lines: vec![base, other],
        ..Invoice::default()
    }
}

fn decide_with(
    inv: &Invoice,
    xml: Option<&[u8]>,
    own: Option<&OwnState>,
    max: Option<i64>,
) -> Decision {
    let cat = catalog();
    decide(
        &Candidate {
            invoice: inv,
            xml,
            own,
        },
        &Rules {
            catalog: &cat,
            max_cents: max,
        },
    )
}

fn pending(d: Decision) -> Pending {
    match d {
        Decision::Pending(p) => *p,
        other => panic!("expected a pending invoice, got {other:?}"),
    }
}

fn recorded(d: Decision) -> super::plan::Recorded {
    match d {
        Decision::Record(r) => r,
        other => panic!("expected a recorded outcome, got {other:?}"),
    }
}

#[test]
fn matching_totals_missing_xml_and_own_state_are_skipped_without_recording() {
    let xml = FIXTURE.as_bytes();
    let same = invoice(["2350", "1700"]);
    match decide_with(&same, Some(xml), None, None) {
        Decision::Skip { line } => {
            assert_eq!(
                line,
                "  skip FVS9000001 SIN_DIFERENCIA XML=4050.00 convenio=4050"
            );
        }
        other => panic!("{other:?}"),
    }
    // 0.01 tolerance is the cents comparison: 4050.004 rounds to 4050.00.
    let near = invoice(["2350", "1700.004"]);
    assert!(matches!(
        decide_with(&near, Some(xml), None, None),
        Decision::Skip { .. }
    ));

    let expected = "  skip FVS9000001 XML_SIN_VALOR (no se pudo extraer el valor total del XML; ver fev verificar)";
    let inv = invoice(["1", "2"]);
    for xml in [None, Some(&b"<no-total/>"[..])] {
        match decide_with(&inv, xml, None, None) {
            Decision::Skip { line } => assert_eq!(line, expected),
            other => panic!("{other:?}"),
        }
    }

    for state in [PhaseState::Excluded, PhaseState::RevisionManual] {
        let own = OwnState {
            state,
            motivo: "BLOQUEADA_MONTO: x".into(),
        };
        match decide_with(&inv, Some(xml), Some(&own), None) {
            Decision::Skip { line } => assert_eq!(
                line,
                format!(
                    "  skip FVS9000001 CONCILIACION {state}: BLOQUEADA_MONTO: x (reprocess with --invoices)"
                )
            ),
            other => panic!("{other:?}"),
        }
    }
    // DONE / FAILED own states do not skip.
    for state in [PhaseState::Done, PhaseState::Failed] {
        let own = OwnState {
            state,
            motivo: String::new(),
        };
        assert!(matches!(
            decide_with(&inv, Some(xml), Some(&own), None),
            Decision::Pending(_)
        ));
    }
}

#[test]
fn an_unreadable_convenio_total_is_not_available_and_does_not_skip() {
    let inv = invoice(["abc", "2"]);
    let p = pending(decide_with(&inv, Some(FIXTURE.as_bytes()), None, None));
    assert_eq!(p.prev_text, "NO_DISPONIBLE");
    assert_eq!(p.xml_text, "4050.00");
}

#[test]
fn a_reconcilable_invoice_waits_for_medicar() {
    let inv = invoice(["1175", "500"]);
    let p = pending(decide_with(
        &inv,
        Some(FIXTURE.as_bytes()),
        None,
        Some(10_000_000),
    ));
    assert_eq!(
        (p.xml_text.as_str(), p.prev_text.as_str()),
        ("4050.00", "1675")
    );
    assert_eq!(p.analysis.state.as_str(), "CONCILIADA");
}

#[test]
fn blocked_difference_and_xml_errors_are_recorded_with_their_states() {
    let inv = invoice(["1175", "500"]);
    let xml = FIXTURE.as_bytes();

    let monto = recorded(decide_with(&inv, Some(xml), None, Some(400_000)));
    assert_eq!(monto.key, CountKey::Bloqueadas);
    assert_eq!(monto.outcome.state, PhaseState::Excluded);
    let summary = "XML=4050.00 convenio_anterior=1675 convenio_conciliado=4050 lineas_xml=3 agregadas=2 no_confirmadas=1";
    let detail = "Valor de factura supera lo permitido: total COP=4050, maximo COP=4000";
    assert_eq!(
        monto.outcome.motivo,
        format!("BLOQUEADA_MONTO: {detail} {summary}")
    );
    assert_eq!(
        monto.line,
        format!("  FVS9000001 BLOQUEADA_MONTO {summary}: {detail}")
    );
    assert_eq!(monto.detail.code, "BLOQUEADA_MONTO");
    assert_eq!(monto.detail.reason, detail);

    let different = FIXTURE.replace(
        "<cbc:PayableAmount currencyID=\"COP\">4050.00",
        "<cbc:PayableAmount currencyID=\"COP\">4100.00",
    );
    let diff = recorded(decide_with(&inv, Some(different.as_bytes()), None, None));
    assert_eq!(
        (diff.key, diff.outcome.state),
        (CountKey::Diferencia, PhaseState::RevisionManual)
    );
    assert!(diff.outcome.motivo.starts_with(
        "DIFERENCIA_PERSISTE: Las lineas XML suman COP=4050, pero el total del XML es COP=4100."
    ));

    let no_lines = "<Invoice><cac:LegalMonetaryTotal><cbc:PayableAmount>10</cbc:PayableAmount></cac:LegalMonetaryTotal></Invoice>";
    let err = recorded(decide_with(&inv, Some(no_lines.as_bytes()), None, None));
    assert_eq!(
        (err.key, err.outcome.state),
        (CountKey::RevisionManual, PhaseState::RevisionManual)
    );
    assert_eq!(
        err.outcome.motivo,
        "ERROR_XML: el XML no contiene lineas de factura conciliables"
    );
    assert_eq!(err.detail.code, "ERROR_XML");

    let unknown = FIXTURE.replace("300_1_1-12345", "300_1_1-55555");
    let err = recorded(decide_with(&inv, Some(unknown.as_bytes()), None, None));
    assert_eq!(
        err.outcome.motivo,
        "ERROR_XML: el PLU-CUM 300_1_1-55555 del XML no cruza con la nota tecnica"
    );

    let no_base = Invoice {
        invoice: "FVS9000001".into(),
        ..Invoice::default()
    };
    let err = recorded(decide_with(&no_base, Some(xml), None, None));
    assert_eq!(
        err.outcome.motivo,
        "ERROR_XML: no hay una linea base del convenio para completar la factura"
    );
}

#[test]
fn cuv_existente_goes_to_manual_review_without_medicar() {
    let r = cuv_existente("FVS9000001");
    assert_eq!(r.outcome.state, PhaseState::RevisionManual);
    assert_eq!(
        r.outcome.motivo,
        "CUV_EXISTENTE: FVS9000001.txt ya tiene CUV con los valores anteriores; requiere revision (nota/anulacion) antes de reconciliar"
    );
    assert_eq!(
        (r.key, r.detail.code.as_str()),
        (CountKey::RevisionManual, "CUV_EXISTENTE")
    );
    assert!(r.detail.reason.starts_with("FVS9000001.txt ya tiene CUV"));
}

fn sample_pending() -> (Invoice, Pending) {
    let inv = invoice(["1175", "500"]);
    let p = pending(decide_with(&inv, Some(FIXTURE.as_bytes()), None, None));
    (inv, p)
}

fn formula() -> Formula {
    Formula {
        fecha_formula: "2026-08-15 10:20:30".into(),
        cantidad: Decimal::from(3),
        ..Formula::default()
    }
}

#[test]
fn a_formula_date_rebuilds_the_invoice_and_marks_both_phases_done() {
    let (inv, p) = sample_pending();
    let now = Utc
        .with_ymd_and_hms(2026, 9, 26, 12, 0, 0)
        .single()
        .expect("time");
    let done = finish_pending(&inv, &p, Ok(formula()), now);
    assert_eq!(done.key, CountKey::Conciliadas);
    assert_eq!(done.conciliacion.state, PhaseState::Done);
    assert_eq!(
        done.conciliacion.motivo,
        "CONCILIADA: La fecha se obtiene de Medicar y las lineas del XML completan exactamente el valor de la factura. XML=4050.00 convenio_anterior=1675 convenio_conciliado=4050 fecha_medicar=2026-08-15 10:20:30 SSC=7770001"
    );
    assert_eq!(done.fev_valor.state, PhaseState::Done);
    assert_eq!(
        done.fev_valor.motivo,
        "CONCILIADA por conciliar: XML=4050.00 convenio=4050"
    );
    assert_eq!(
        done.line,
        "  FVS9000001 CONCILIADA XML=4050.00 convenio_anterior=1675 convenio_conciliado=4050 fecha_medicar=2026-08-15 10:20:30"
    );
    let rebuilt = done.rebuilt.expect("rebuilt");
    assert_eq!(rebuilt.lines.len(), 3);
    let audit = rebuilt.conciliacion.expect("audit");
    assert_eq!(
        (audit.ssc.as_str(), audit.formula_date.as_str()),
        ("7770001", "2026-08-15 10:20:30")
    );
    assert_eq!(audit.reconciled_at, Some(now));
    assert!(done.detail.is_none());
}

#[test]
fn medicar_failures_map_to_states() {
    let (inv, p) = sample_pending();
    let now = Utc::now();

    let permanent = MedicarError::Permanent(
        "medicar: ninguna molecula de Medicar coincide con el articulo del convenio".into(),
    );
    let f = finish_pending(&inv, &p, Err(permanent), now);
    assert_eq!(
        (f.key, f.conciliacion.state, f.fev_valor.state),
        (
            CountKey::RevisionManual,
            PhaseState::RevisionManual,
            PhaseState::RevisionManual
        )
    );
    assert_eq!(
        f.conciliacion.motivo,
        "MEDICAR: medicar: ninguna molecula de Medicar coincide con el articulo del convenio"
    );
    assert_eq!(f.fev_valor.motivo, f.conciliacion.motivo);
    assert_eq!(f.line, format!("  FVS9000001 {}", f.conciliacion.motivo));
    let d = f.detail.expect("detail");
    assert_eq!(
        (d.code.as_str(), d.failed, d.unavailable),
        ("MEDICAR", false, false)
    );
    assert!(f.rebuilt.is_none());

    let down = MedicarError::Unavailable("medicar: GET /: connection refused".into());
    let f = finish_pending(&inv, &p, Err(down), now);
    assert_eq!(
        (f.key, f.conciliacion.state),
        (CountKey::Fallidas, PhaseState::Failed)
    );
    assert_eq!(
        f.conciliacion.motivo,
        "MEDICAR no disponible: medicar: GET /: connection refused; se reintentara en la proxima ejecucion"
    );
    let d = f.detail.expect("detail");
    assert_eq!((d.failed, d.unavailable), (true, true));
    assert!(d.reason.starts_with("no disponible: "));

    let other = MedicarError::Transient("medicar: GET /: read body: reset".into());
    let f = finish_pending(&inv, &p, Err(other), now);
    assert_eq!(
        (f.key, f.conciliacion.state),
        (CountKey::Fallidas, PhaseState::Failed)
    );
    assert_eq!(
        f.conciliacion.motivo,
        "MEDICAR: medicar: GET /: read body: reset"
    );
    assert_eq!(
        f.detail.map(|d| (d.failed, d.unavailable)),
        Some((true, false))
    );
}

#[test]
fn summary_line_and_exit_rules() {
    let mut c = Counts::default();
    for key in [
        CountKey::Conciliadas,
        CountKey::Conciliadas,
        CountKey::Omitidas,
    ] {
        c.add(key);
    }
    assert_eq!(
        c.summary_line(),
        "conciliar: conciliadas=2 bloqueadas=0 diferencia=0 revision_manual=0 fallidas=0 omitidas=1"
    );
    assert!(
        c.incomplete(vec![]).is_none(),
        "reconciled and skipped invoices exit 0"
    );
    for key in [
        CountKey::Bloqueadas,
        CountKey::Diferencia,
        CountKey::RevisionManual,
        CountKey::Fallidas,
    ] {
        let mut c = Counts::default();
        c.add(key);
        assert!(c.incomplete(vec![]).is_some(), "{key:?} exits non-zero");
    }
}

fn detail(i: usize, failed: bool, unavailable: bool) -> Detail {
    Detail {
        invoice: format!("F{i}"),
        code: "MEDICAR".into(),
        reason: format!("motivo   {i}\ncon  saltos"),
        failed,
        unavailable,
    }
}

#[test]
fn the_error_lists_the_first_five_details_with_cut_reasons() {
    let long = Detail {
        reason: "x".repeat(300),
        ..detail(0, false, false)
    };
    let err = Incomplete {
        manual_review: 1,
        failed: 0,
        details: vec![long],
    };
    let text = err.to_string();
    assert!(
        text.starts_with("1 factura(s) sin conciliar: F0 → MEDICAR (revisión manual): "),
        "{text}"
    );
    assert!(text.contains(&format!("{}…", "x".repeat(240))), "{text}");
    assert!(text.ends_with(". Motivo completo en la página «Revisión manual» o en el log."));

    let seven: Vec<_> = (1..=7).map(|i| detail(i, i % 2 == 0, false)).collect();
    let err = Incomplete {
        manual_review: 4,
        failed: 3,
        details: seven,
    };
    let text = err.to_string();
    assert!(text.starts_with("7 factura(s) sin conciliar: F1 → MEDICAR (revisión manual): motivo 1 con saltos; F2 → MEDICAR (fallo técnico, reintentar): motivo 2 con saltos;"), "{text}");
    assert!(text.contains("; y 2 más. Motivo completo"), "{text}");
    assert!(!text.contains("F6"), "{text}");

    let bare = Incomplete {
        manual_review: 0,
        failed: 2,
        details: vec![],
    };
    assert_eq!(bare.to_string(), "2 factura(s) sin conciliar");
}

#[test]
fn only_a_medicar_outage_lets_proceso_continue() {
    let down = Incomplete {
        manual_review: 0,
        failed: 2,
        details: vec![detail(1, true, true), detail(2, true, true)],
    };
    assert!(down.only_medicar_unavailable());
    let mixed = Incomplete {
        manual_review: 0,
        failed: 2,
        details: vec![detail(1, true, true), detail(2, true, false)],
    };
    assert!(!mixed.only_medicar_unavailable());
    let manual = Incomplete {
        manual_review: 1,
        failed: 1,
        details: vec![detail(1, true, true)],
    };
    assert!(!manual.only_medicar_unavailable());
    assert!(
        !Incomplete {
            manual_review: 1,
            failed: 0,
            details: vec![]
        }
        .only_medicar_unavailable()
    );
}

#[test]
fn one_line_collapses_and_cuts() {
    assert_eq!(one_line("a \n  b\t c", 10), "a b c");
    assert_eq!(one_line("abcdef", 3), "abc…");
    assert_eq!(one_line("ñandú", 5), "ñandú");
}
