//! Read models B of the dashboard API (hallazgos, informe, radicadas, portal),
//! through `ApiState::handle` against a throwaway Postgres. Nothing here
//! touches a real external system.

#[path = "api_read_b_support.rs"]
mod support;

use serde_json::{Value, json};
use support::*;
use tessia::phases::radicacion::portal::{Finding, PortalStore, Record, Snapshot};

// ---- hallazgos -------------------------------------------------------------

fn finding(code: &str, detail: &str) -> Finding {
    Finding {
        code: code.into(),
        kind: "ADVERTENCIA".into(),
        field: "campo".into(),
        detail: detail.into(),
    }
}

fn save_findings(env: &Env, invoice: &str, findings: Vec<Finding>) {
    PortalStore::new(env.db.pool())
        .save(&Snapshot {
            record: Record {
                invoice: invoice.into(),
                status: "VALIDADO_CON_ADVERTENCIA".into(),
                ..Record::default()
            },
            findings,
            found: true,
            queries: 1,
            ..Snapshot::default()
        })
        .expect("save portal snapshot");
}

#[test]
fn the_catalog_lists_the_followed_findings_ordered_by_code_without_caching() {
    let env = env();
    let empty = env.get("/api/hallazgos", &[]);
    assert_eq!(empty.status, 200);
    assert_eq!(json(&empty), json!({"hallazgos": [], "total": 0}));

    save_findings(
        &env,
        "FEV1",
        vec![finding("RVC012", "primero"), finding("AAA001", "otro")],
    );
    let response = env.get("/api/hallazgos", &[]);
    assert_eq!(response.status, 200);
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(
        response.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(
        json(&response),
        json!({"hallazgos": [
            {"codigo": "AAA001", "detalle": "otro", "genera_glosa": false, "devolucion": false, "descripcion": ""},
            {"codigo": "RVC012", "detalle": "primero", "genera_glosa": false, "devolucion": false, "descripcion": ""},
        ], "total": 2})
    );
}

#[test]
fn updating_a_finding_stores_the_edit_and_keeps_devolucion_when_absent() {
    let env = env();
    save_findings(&env, "FEV1", vec![finding("RVC012", "primero")]);

    let first = env.put_json(
        "/api/hallazgos/RVC012",
        r#"{"genera_glosa":true,"devolucion":true,"descripcion":"  Falta soporte  "}"#,
    );
    assert_eq!(first.status, 200);
    assert_eq!(first.header("cache-control"), Some("no-store"));
    assert_eq!(
        json(&first),
        json!({"codigo": "RVC012", "detalle": "primero", "genera_glosa": true, "devolucion": true, "descripcion": "Falta soporte"})
    );

    // Absent or null devolucion keeps the stored value.
    for body in [
        r#"{"genera_glosa":false,"descripcion":""}"#,
        r#"{"genera_glosa":false,"devolucion":null,"descripcion":""}"#,
    ] {
        let again = env.put_json("/api/hallazgos/RVC012", body);
        assert_eq!(again.status, 200, "{body}");
        let view = json(&again);
        assert_eq!(view["devolucion"], Value::Bool(true), "{body}");
        assert_eq!(view["genera_glosa"], Value::Bool(false), "{body}");
        assert_eq!(view["descripcion"], json!(""), "{body}");
    }

    // The edit is persisted and visible in the list.
    let listed = json(&env.get("/api/hallazgos", &[]));
    assert_eq!(listed["hallazgos"][0]["devolucion"], Value::Bool(true));
    // Go matches keys case-insensitively.
    let cased = env.put_json(
        "/api/hallazgos/RVC012",
        r#"{"Genera_Glosa":true,"Descripcion":"x"}"#,
    );
    assert_eq!(cased.status, 200);
    assert_eq!(json(&cased)["descripcion"], json!("x"));
}

#[test]
fn an_invalid_update_is_rejected_with_the_spanish_messages_and_changes_nothing() {
    let env = env();
    save_findings(&env, "FEV1", vec![finding("RVC012", "primero")]);
    let put = |code: &str, body: &str| env.put_json(&format!("/api/hallazgos/{code}"), body);
    let before = json(&env.get("/api/hallazgos", &[]));

    let cases: Vec<(String, String, u16, &str)> = vec![
        (
            "a b".into(),
            "{}".into(),
            400,
            "Código de hallazgo inválido.",
        ),
        (
            "A".repeat(101),
            "{}".into(),
            400,
            "Código de hallazgo inválido.",
        ),
        (
            "RVC012".into(),
            "{}".into(),
            400,
            "Falta el campo genera_glosa.",
        ),
        (
            "RVC012".into(),
            r#"{"devolucion":true}"#.into(),
            400,
            "Falta el campo genera_glosa.",
        ),
        (
            "RVC012".into(),
            r#"{"genera_glosa":true,"otro":1}"#.into(),
            400,
            "El cuerpo debe ser un JSON válido con genera_glosa y descripcion.",
        ),
        (
            "RVC012".into(),
            "no es json".into(),
            400,
            "El cuerpo debe ser un JSON válido con genera_glosa y descripcion.",
        ),
        (
            "RVC012".into(),
            r#"{"genera_glosa":true}{"genera_glosa":false}"#.into(),
            400,
            "El cuerpo debe contener un único objeto JSON.",
        ),
        (
            "RVC012".into(),
            json!({"genera_glosa": true, "descripcion": "a".repeat(2001)}).to_string(),
            400,
            "La descripción no puede superar 2000 caracteres.",
        ),
        (
            "NOEXISTE".into(),
            r#"{"genera_glosa":true}"#.into(),
            404,
            "El hallazgo no existe.",
        ),
    ];
    for (code, body, status, message) in cases {
        let response = put(&code, &body);
        assert_eq!(response.status, status, "{code} {body}");
        assert_eq!(error(&response), message, "{code} {body}");
    }

    // The rejected requests wrote nothing.
    assert_eq!(json(&env.get("/api/hallazgos", &[])), before);

    // 2000 characters (counted after trimming, as characters) are accepted.
    let edge = json!({"genera_glosa": true, "descripcion": format!(" {} ", "ñ".repeat(2000))});
    assert_eq!(put("RVC012", &edge.to_string()).status, 200);
    let stored = &json(&env.get("/api/hallazgos", &[]))["hallazgos"][0];
    assert_eq!(
        stored["descripcion"].as_str().map(|s| s.chars().count()),
        Some(2000)
    );
}

#[test]
fn hallazgos_routes_answer_405_for_other_methods() {
    let env = env();
    let response = env.post("/api/hallazgos");
    assert_eq!(response.status, 405);
    assert_eq!(response.header("allow"), Some("GET"));
}

// ---- informe ---------------------------------------------------------------

use tessia::domain::Outcome;
use tessia::domain::Phase;
use tessia::domain::dataset::test_support::InvoiceBuilder;

const CARGUE: &str = "CargueA";

/// The synthetic equivalent of Go's `testdata/golden/dashboard/informe_a.json`.
fn informe_fixture(env: &Env) {
    env.dataset(CARGUE, 'a', |ds| {
        ds.invoice(InvoiceBuilder::new("FVS1").line("10"))
            .invoice(InvoiceBuilder::new("FVS2").line("10"))
            .invoice(InvoiceBuilder::new("fvs-3").line("10"))
            .invoice(InvoiceBuilder::new("FVS6").line("10"))
            .excluded("FVS8", "MONTO_MAXIMO_SUPERADO")
            .manual_review("FVS9", "SIN_SSC", "sin ssc")
    });
    env.phase(
        "FVS6",
        Phase::Derechos,
        Outcome::manual_review("SIN_DERECHOS: no vigente"),
        13,
    );
    env.phase(
        "FVS3",
        Phase::TheFactory,
        Outcome::manual_review("SIN_XML: falta"),
        11,
    );
    env.phase(
        "FVS2",
        Phase::Rips,
        Outcome::failed("DESCARGA: timeout"),
        12,
    );
    env.phase(
        "FVS3",
        Phase::FevValor,
        Outcome::manual_review("VALOR_DIFERENTE: x"),
        10,
    );
    env.phase(
        "FVS3",
        Phase::Conciliacion,
        Outcome::excluded("BLOQUEADA_TARIFA: precio del XML supera la nota tecnica"),
        20,
    );
    env.phase("FVS1", Phase::Derechos, Outcome::done(), 5);
    env.phase(
        "FVS1",
        Phase::Cuv,
        Outcome::new(tessia::domain::PhaseState::EnProgreso, ""),
        6,
    );
}

#[test]
fn the_informe_lists_what_stopped_in_the_golden_shape() {
    let env = env();
    informe_fixture(&env);
    let response = env.get(&format!("/api/cargues/{CARGUE}/informe"), &[]);
    assert_eq!(response.status, 200);
    assert_eq!(
        response.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    assert!(response.body.ends_with(b"\n"));
    let mut body = json(&response);
    let generated = body["generado_en"]
        .as_str()
        .expect("generado_en")
        .to_owned();
    assert!(
        generated.ends_with('Z') && generated.len() == 20,
        "{generated}"
    );
    body["generado_en"] = json!("GENERATED");
    assert_eq!(
        body,
        json!({
            "cargue": CARGUE,
            "generado_en": "GENERATED",
            "incompleto": false,
            "conteos": {"revision_manual": 3, "excluidas": 2, "fallas": 1},
            "items": [
                {"factura": "FVS9", "fase": "convenio", "categoria": "revision_manual", "codigo": "SIN_SSC", "motivo": "sin ssc", "actualizado_en": ""},
                {"factura": "FVS6", "fase": "derechos", "categoria": "revision_manual", "codigo": "SIN_DERECHOS", "motivo": "no vigente", "actualizado_en": "2026-09-12T07:13:00Z"},
                {"factura": "FVS3", "fase": "thefactory", "categoria": "revision_manual", "codigo": "SIN_XML", "motivo": "falta", "actualizado_en": "2026-09-12T07:11:00Z"},
                {"factura": "FVS2", "fase": "rips", "categoria": "falla", "codigo": "DESCARGA", "motivo": "timeout", "actualizado_en": "2026-09-12T07:12:00Z"},
                {"factura": "FVS8", "fase": "convenio", "categoria": "excluida", "codigo": "", "motivo": "MONTO_MAXIMO_SUPERADO", "actualizado_en": ""},
                {"factura": "FVS3", "fase": "conciliacion", "categoria": "excluida", "codigo": "BLOQUEADA_TARIFA", "motivo": "precio del XML supera la nota tecnica", "actualizado_en": "2026-09-12T07:20:00Z"},
            ]
        })
    );
}

#[test]
fn the_informe_csv_is_a_download_with_bom_semicolons_and_neutralized_formulas() {
    let env = env();
    env.dataset("Carga_v2-b", 'a', |ds| {
        ds.invoice(InvoiceBuilder::new("FVS1").line("10"))
            .manual_review("FVS9", "SIN_SSC", "=HYPERLINK(\"x\");y")
    });
    env.phase(
        "FVS1",
        Phase::Rips,
        Outcome::failed("DESCARGA: timeout"),
        12,
    );
    let response = env.get("/api/cargues/Carga_v2-b/informe", &[("formato", "csv")]);
    assert_eq!(response.status, 200);
    assert_eq!(
        response.header("content-type"),
        Some("text/csv; charset=utf-8")
    );
    assert_eq!(
        response.header("content-disposition"),
        Some("attachment; filename=\"Informe_revision_manual_Carga_v2-b.csv\"")
    );
    assert_eq!(
        text(&response),
        "\u{feff}Factura;Fase;Categoria;Codigo;Motivo;Actualizado\r\n\
         FVS9;convenio;Revisión manual;SIN_SSC;\"'=HYPERLINK(\"\"x\"\");y\";\r\n\
         FVS1;rips;Falla;DESCARGA;timeout;2026-09-12T07:12:00Z\r\n"
    );
}

#[test]
fn the_informe_reads_the_current_cargue_and_an_unprepared_one_is_empty() {
    let env = env();
    informe_fixture(&env);
    // A newer upload of the same load, not prepared yet, does not shadow the
    // prepared dataset the pointer names.
    tessia::stores::dataset::test_support::seed_cargue(&env.db.pool(), CARGUE, 'b');
    let current = json(&env.get(&format!("/api/cargues/{CARGUE}/informe"), &[]));
    assert_eq!(current["conteos"]["revision_manual"], json!(3));

    // A load that was only uploaded has an empty report.
    tessia::stores::dataset::test_support::seed_cargue(&env.db.pool(), "SoloCsv", 'c');
    let empty = env.get("/api/cargues/SoloCsv/informe", &[]);
    assert_eq!(empty.status, 200);
    let body = json(&empty);
    assert_eq!(body["items"], json!([]));
    assert_eq!(body["incompleto"], json!(false));
    assert_eq!(
        body["conteos"],
        json!({"revision_manual": 0, "excluidas": 0, "fallas": 0})
    );
}

#[test]
fn the_informe_rejects_a_bad_format_and_unknown_cargues() {
    let env = env();
    informe_fixture(&env);
    let bad = env.get(
        &format!("/api/cargues/{CARGUE}/informe"),
        &[("formato", "xml")],
    );
    assert_eq!(
        (bad.status, error(&bad).as_str()),
        (400, "formato debe ser json o csv")
    );
    // The format is checked before the cargue.
    let both = env.get("/api/cargues/Nope/informe", &[("formato", "xml")]);
    assert_eq!(both.status, 400);

    let unknown = env.get("/api/cargues/Nope/informe", &[]);
    assert_eq!(unknown.status, 404);
    assert_eq!(error(&unknown), "dashboard: cargue not found: \"Nope\"");
    let dots = env.get("/api/cargues/../informe", &[]);
    assert_eq!(dots.status, 404);
    assert_eq!(
        error(&dots),
        "dashboard: cargue not found: invalid id \"..\""
    );
    // Only GET is routed.
    assert_eq!(
        env.post(&format!("/api/cargues/{CARGUE}/informe")).status,
        405
    );
}

// ---- radicadas -------------------------------------------------------------

use chrono::{Local, TimeZone, Utc};
use tessia::domain::dataset::Line;
use tessia::domain::dataset::test_support::sample_line;
use tessia::domain::radicacion::{RadStatus, Step};

fn line(invoice: &str, date: &str, ssc: &str, authorization: &str, total: &str) -> Line {
    Line {
        date: date.into(),
        ssc: ssc.into(),
        authorization: authorization.into(),
        ..sample_line(invoice, 1, total)
    }
}

fn filed(env: &Env, invoice: &str, at: &str, detail: &str) {
    env.filing(invoice, Step::Crc, RadStatus::Confirmed, detail, at);
}

/// Two cargues; `FEV3` stopped at step 3, `FEV4` was never queued and `FEV5`
/// is excluded although it has a confirmed filing row.
fn radicadas_fixture(env: &Env) {
    env.dataset("CargueA", 'a', |ds| {
        ds.invoice(
            InvoiceBuilder::new("FEV1")
                .with_line(line("FEV1", "2026-09-03 11:28:31", "SSC1", "", "1.000,50"))
                .with_line(line("FEV1", "", "", "AUT-9", "499,50")),
        )
        .invoice(InvoiceBuilder::new("FEV2").with_line(line(
            "FEV2",
            "2026-09-04",
            "SSC2",
            "AUT2",
            "10",
        )))
        .invoice(InvoiceBuilder::new("FEV3").line("5"))
        .invoice(InvoiceBuilder::new("FEV4").line("5"))
        .excluded("FEV5", "MONTO_MAXIMO_SUPERADO")
    });
    env.dataset("CargueB", 'b', |ds| {
        ds.invoice(InvoiceBuilder::new("FEV6").with_line(line("FEV6", "", "", "", "abc")))
            .invoice(InvoiceBuilder::new("FEV10").line("7"))
    });
    filed(env, "FEV1", "2026-09-14 15:00:00+00", "Confirmado   paso 4");
    filed(env, "FEV2", "2026-09-15 15:00:00+00", "");
    filed(env, "FEV6", "2026-09-15 15:00:00+00", "=SUMA(1);2");
    filed(env, "FEV10", "2026-09-10 15:00:00+00", "");
    filed(env, "FEV5", "2026-09-16 15:00:00+00", "");
    env.filing(
        "FEV3",
        Step::Rips,
        RadStatus::Confirmed,
        "",
        "2026-09-16 15:00:00+00",
    );
    env.phase(
        "FEV1",
        Phase::Cuv,
        Outcome::failed("CUV: intento 1\n  falló"),
        7,
    );
    env.phase("FEV1", Phase::Rips, Outcome::done(), 8);
}

fn invoices(body: &Value) -> Vec<String> {
    body["filas"]
        .as_array()
        .expect("filas")
        .iter()
        .map(|r| r["factura"].as_str().expect("factura").to_owned())
        .collect()
}

#[test]
fn the_radicadas_report_lists_only_fully_filed_prepared_invoices_with_their_summary() {
    let env = env();
    radicadas_fixture(&env);
    let response = env.get("/api/reportes/radicadas", &[]);
    assert_eq!(response.status, 200);
    assert_eq!(
        response.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    let mut body = json(&response);
    assert_eq!(body["generado_en"].as_str().map(str::len), Some(20));
    body["generado_en"] = json!("GENERATED");
    assert_eq!(
        body,
        json!({
            "generado_en": "GENERATED",
            "total": 4,
            "truncado": false,
            "filas": [
                {"cargue": "CargueA", "cargue_nombre": "CargueA", "factura": "FEV2", "fecha_radicacion": "2026-09-15T15:00:00Z",
                 "estado": "TERMINADO", "fecha_factura": "2026-09-04", "ssc": "SSC2", "autorizacion": "AUT2", "valor": "10", "observaciones": ""},
                {"cargue": "CargueB", "cargue_nombre": "CargueB", "factura": "FEV6", "fecha_radicacion": "2026-09-15T15:00:00Z",
                 "estado": "TERMINADO", "fecha_factura": "", "ssc": "", "autorizacion": "", "valor": "", "observaciones": "=SUMA(1);2"},
                {"cargue": "CargueA", "cargue_nombre": "CargueA", "factura": "FEV1", "fecha_radicacion": "2026-09-14T15:00:00Z",
                 "estado": "TERMINADO", "fecha_factura": "2026-09-03", "ssc": "SSC1", "autorizacion": "AUT-9", "valor": "1500",
                 "observaciones": "CUV: intento 1 falló | Confirmado paso 4"},
                {"cargue": "CargueB", "cargue_nombre": "CargueB", "factura": "FEV10", "fecha_radicacion": "2026-09-10T15:00:00Z",
                 "estado": "TERMINADO", "fecha_factura": "2026-10-21", "ssc": "", "autorizacion": "", "valor": "7", "observaciones": ""},
            ]
        })
    );
    // Personal data of the lines never reaches the report.
    assert!(!text(&response).contains("1000000001"));
}

#[test]
fn the_radicadas_filters_narrow_by_cargue_day_and_invoice_text() {
    let env = env();
    radicadas_fixture(&env);
    let list = |query: &[(&str, &str)]| invoices(&json(&env.get("/api/reportes/radicadas", query)));

    assert_eq!(list(&[("cargue", "CargueA")]), ["FEV2", "FEV1"]);
    assert_eq!(
        list(&[("cargue", "CargueB"), ("cargue", "CargueB")]),
        ["FEV6", "FEV10"]
    );
    assert_eq!(
        list(&[("cargue", "CargueB"), ("cargue", "CargueA"), ("cargue", "")]),
        ["FEV2", "FEV6", "FEV1", "FEV10"]
    );
    // `q` is a substring of the normalized invoice, in any case, ignoring hyphens.
    assert_eq!(list(&[("q", "fev-1")]), ["FEV1", "FEV10"]);
    assert_eq!(list(&[("q", " ev6 ")]), ["FEV6"]);
    assert!(list(&[("q", "ZZZ")]).is_empty());

    // Days compare the local day of the filing time, bounds included.
    let local_day = |day: u32| {
        Utc.with_ymd_and_hms(2026, 9, day, 15, 0, 0)
            .unwrap()
            .with_timezone(&Local)
            .format("%Y-%m-%d")
            .to_string()
    };
    assert_eq!(
        list(&[("desde", &local_day(14)), ("hasta", &local_day(14))]),
        ["FEV1"]
    );
    assert_eq!(list(&[("desde", &local_day(15))]), ["FEV2", "FEV6"]);
    assert_eq!(list(&[("hasta", &local_day(10))]), ["FEV10"]);
    assert_eq!(
        list(&[
            ("desde", &local_day(11)),
            ("hasta", &local_day(14)),
            ("q", "fev")
        ]),
        ["FEV1"]
    );
}

#[test]
fn limite_truncates_the_preview_but_total_counts_every_match() {
    let env = env();
    radicadas_fixture(&env);
    let body = json(&env.get("/api/reportes/radicadas", &[("limite", "3")]));
    assert_eq!(body["total"], json!(4));
    assert_eq!(body["truncado"], json!(true));
    assert_eq!(invoices(&body), ["FEV2", "FEV6", "FEV1"]);
    let exact = json(&env.get("/api/reportes/radicadas", &[("limite", "4")]));
    assert_eq!(
        (exact["total"].clone(), exact["truncado"].clone()),
        (json!(4), json!(false))
    );
    // The CSV ignores the limit.
    let csv = env.get(
        "/api/reportes/radicadas",
        &[("limite", "1"), ("formato", "csv")],
    );
    assert_eq!(
        text(&csv).matches("\r\n").count(),
        5,
        "header plus every row"
    );
}

#[test]
fn an_empty_database_reports_nothing() {
    let env = env();
    let body = json(&env.get("/api/reportes/radicadas", &[]));
    assert_eq!(
        (
            body["total"].clone(),
            body["truncado"].clone(),
            body["filas"].clone()
        ),
        (json!(0), json!(false), json!([]))
    );
}

#[test]
fn the_radicadas_query_is_validated_before_any_read() {
    let env = env();
    radicadas_fixture(&env);
    let reject = |query: &[(&str, &str)], status: u16, message: &str| {
        let response = env.get("/api/reportes/radicadas", query);
        assert_eq!(
            (response.status, error(&response)),
            (status, message.to_owned()),
            "{query:?}"
        );
    };
    reject(&[("formato", "xml")], 400, "formato debe ser json o csv");
    reject(&[("cargue", "a\\b")], 400, "cargue invalido \"a\\\\b\"");
    reject(
        &[("cargue", "Nope")],
        404,
        "dashboard: cargue not found: \"Nope\"",
    );
    reject(&[("desde", "2026-9-1")], 400, "desde debe ser YYYY-MM-DD");
    reject(&[("hasta", "mañana")], 400, "hasta debe ser YYYY-MM-DD");
    reject(
        &[("desde", "2026-09-02"), ("hasta", "2026-09-01")],
        400,
        "hasta es anterior a desde",
    );
    reject(
        &[("q", &"x".repeat(61))],
        400,
        "q no puede superar 60 caracteres",
    );
    reject(
        &[("limite", "0")],
        400,
        "limite debe ser un entero positivo",
    );
    reject(
        &[("limite", "tres")],
        400,
        "limite debe ser un entero positivo",
    );
}

#[test]
fn the_radicadas_csv_is_a_named_download_with_local_filing_times() {
    let env = env();
    radicadas_fixture(&env);
    let today = Local::now().format("%Y%m%d").to_string();
    let name = |query: &[(&str, &str)]| {
        let mut all = query.to_vec();
        all.push(("formato", "csv"));
        let response = env.get("/api/reportes/radicadas", &all);
        assert_eq!(response.status, 200);
        assert_eq!(
            response.header("content-type"),
            Some("text/csv; charset=utf-8")
        );
        response
            .header("content-disposition")
            .expect("content-disposition")
            .to_owned()
    };
    assert_eq!(
        name(&[]),
        format!("attachment; filename=\"radicadas-todos-{today}.csv\"")
    );
    assert_eq!(
        name(&[("cargue", "CargueA")]),
        format!("attachment; filename=\"radicadas-CargueA-{today}.csv\"")
    );
    assert_eq!(
        name(&[("cargue", "CargueA"), ("cargue", "CargueB")]),
        format!("attachment; filename=\"radicadas-varios-{today}.csv\"")
    );

    let response = env.get(
        "/api/reportes/radicadas",
        &[("formato", "csv"), ("q", "fev6")],
    );
    let local = Utc
        .with_ymd_and_hms(2026, 9, 15, 15, 0, 0)
        .unwrap()
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    assert_eq!(
        text(&response),
        format!(
            "\u{feff}Cargue;Nombre cargue;Factura;Fecha factura;Fecha radicacion;Estado;SSC;Autorizacion;Valor convenio;Observaciones\r\n\
             CargueB;CargueB;FEV6;;{local};TERMINADO;;;;\"'=SUMA(1);2\"\r\n"
        )
    );
}

#[test]
fn only_get_is_routed_for_the_radicadas_report() {
    let env = env();
    let response = env.post("/api/reportes/radicadas");
    assert_eq!(response.status, 405);
}

// ---- portal ----------------------------------------------------------------

use std::sync::{Arc, Mutex};

use tessia::testing::{MockResponse, MockServer, RecordedRequest};

/// How the scripted Mutual Ser portal answers `core/filtersAll`.
#[derive(Clone)]
enum Portal {
    Row(Value),
    Empty,
    Fail(&'static str),
}

fn portal_server(portal: Arc<Mutex<Portal>>) -> MockServer {
    MockServer::start(move |req: &RecordedRequest| {
        let path = req.path.split('?').next().unwrap_or_default();
        match path {
            "/login" => MockResponse::json(200, r#"{"access_token":"tok","expires_in":3600}"#),
            "/rips-api/core/filtersAll" => match portal.lock().expect("portal").clone() {
                Portal::Row(row) => {
                    MockResponse::json(200, &json!({"contenido": [row]}).to_string())
                }
                Portal::Empty => MockResponse::json(200, r#"{"contenido":[]}"#),
                Portal::Fail(body) => MockResponse::text(400, body),
            },
            p if p.starts_with("/rips-api/core/findDetailsByFile/") => MockResponse::json(
                200,
                r#"[{"codigo":"A1","tipo":"ADVERTENCIA","campo":"edad","detalle":"revisar"},
                    {"codigo":"E9","tipo":"ERROR","campo":"","detalle":"falta"}]"#,
            ),
            _ => MockResponse::text(404, ""),
        }
    })
}

fn portal_row(invoice: &str, status: &str) -> Value {
    json!({
        "numero_factura": invoice, "estado": status, "remision": "R7", "usuario": "ops@example.org",
        "servicios": "[1]", "modalidad": "EVENTO", "nombreArchivo": format!("{invoice}.json"),
        "idArchivo": format!("f-{invoice}"), "idCargue": "c1", "tipoEvento": "REGISTER_RIP",
        "fecha_cargue": "29-09-2026 17:01:26", "documento_soporte": {"id": "s1", "estado": "ACTIVE"}
    })
}

fn portal_env(portal: &Arc<Mutex<Portal>>) -> (Env, MockServer) {
    let server = portal_server(portal.clone());
    let url = server.url().to_owned();
    let env = env_with(vec![
        ("TESSIA_ZONASER_LOGIN_URL", format!("{url}/login")),
        (
            "TESSIA_ZONASER_BENAVENTE_BASE_URL",
            format!("{url}/rips-api"),
        ),
        ("TESSIA_ZONASER_USERNAME", "user".to_owned()),
        ("TESSIA_ZONASER_PASSWORD", "secret".to_owned()),
        ("TESSIA_ZONASER_ORGANIZATION_NIT", "900111222".to_owned()),
    ]);
    (env, server)
}

fn portal_calls(server: &MockServer) -> usize {
    server
        .requests()
        .iter()
        .filter(|r| r.path.ends_with("/core/filtersAll"))
        .count()
}

#[test]
fn a_first_lookup_queries_the_portal_stores_the_result_and_then_serves_it_from_the_database() {
    let portal = Arc::new(Mutex::new(Portal::Row(portal_row(
        "FEV1",
        "VALIDADO_CON_ADVERTENCIA",
    ))));
    let (env, server) = portal_env(&portal);

    let first = env.get("/api/facturas/fev-1/portal", &[]);
    assert_eq!(first.status, 200);
    assert_eq!(first.header("cache-control"), Some("no-store"));
    let mut body = json(&first);
    let queried = body["consultado_en"]
        .as_str()
        .expect("consultado_en")
        .to_owned();
    assert!(queried.ends_with('Z'), "{queried}");
    body["consultado_en"] = json!("NOW");
    assert_eq!(
        body,
        json!({
            "factura": "FEV1", "consultado": true, "final": true, "estado": "VALIDADO_CON_ADVERTENCIA",
            "remision": "R7", "fecha_cargue": "2026-09-29T17:01:26-05:00", "fecha_cargue_texto": "29-09-2026 17:01:26",
            "usuario": "ops@example.org", "servicios": "[1]", "modalidad": "EVENTO", "nombre_archivo": "FEV1.json",
            "id_archivo": "f-FEV1", "id_cargue": "c1", "id_evento": "", "tipo_evento": "REGISTER_RIP", "tipo_error": "",
            "codigo_error": "", "devuelto": false, "fecha_expedicion": "", "receipt_name": "",
            "documento_soporte": {"id": "s1", "estado": "ACTIVE"}, "advertencias": 1, "errores": 1,
            "hallazgos": [
                {"codigo": "A1", "tipo": "ADVERTENCIA", "campo": "edad", "detalle": "revisar"},
                {"codigo": "E9", "tipo": "ERROR", "campo": "", "detalle": "falta"}
            ],
            "consultas": 1, "consultado_en": "NOW", "origen": "portal", "mensaje": "", "error": ""
        })
    );
    assert_eq!(portal_calls(&server), 1);

    // Final and stored: served from the database, no new portal call.
    let again = json(&env.get("/api/facturas/FEV1/portal", &[]));
    assert_eq!(again["origen"], json!("base_de_datos"));
    assert_eq!(again["consultas"], json!(1));
    assert_eq!(again["hallazgos"].as_array().map(Vec::len), Some(2));
    assert_eq!(portal_calls(&server), 1);

    // The findings fed the catalog behind /api/hallazgos.
    let catalog = json(&env.get("/api/hallazgos", &[]));
    assert_eq!(catalog["total"], json!(2));

    // POST forces a query even when the stored result is final.
    let refreshed = env.post("/api/facturas/FEV1/portal/actualizar");
    assert_eq!(refreshed.status, 200);
    assert_eq!(refreshed.header("cache-control"), Some("no-store"));
    let refreshed = json(&refreshed);
    assert_eq!(
        (refreshed["origen"].clone(), refreshed["consultas"].clone()),
        (json!("portal"), json!(2))
    );
    assert_eq!(portal_calls(&server), 2);
}

#[test]
fn a_portal_failure_is_a_200_with_a_scrubbed_error_and_keeps_the_stored_data() {
    let portal = Arc::new(Mutex::new(Portal::Row(portal_row("FEV1", "EN PROCESO"))));
    let (env, server) = portal_env(&portal);
    let stored = json(&env.get("/api/facturas/FEV1/portal", &[]));
    assert_eq!(stored["final"], json!(false));

    *portal.lock().expect("portal") = Portal::Fail(
        "denied for ops@example.org at https://zonaser.example.org/x with token=abc123 Bearer xyz",
    );
    let response = env.post("/api/facturas/FEV1/portal/actualizar");
    assert_eq!(response.status, 200);
    let body = json(&response);
    let message = body["error"].as_str().expect("error");
    assert!(
        message.starts_with("No se pudo consultar el portal: "),
        "{message}"
    );
    for secret in ["ops@example.org", "zonaser.example.org", "abc123", "xyz"] {
        assert!(!message.contains(secret), "{message}");
    }
    assert!(message.chars().count() <= "No se pudo consultar el portal: ".chars().count() + 200);
    // What was stored stays in the answer.
    assert_eq!(
        (
            body["consultado"].clone(),
            body["estado"].clone(),
            body["origen"].clone()
        ),
        (json!(true), json!("EN PROCESO"), json!("base_de_datos"))
    );
    assert!(portal_calls(&server) >= 2);
}

#[test]
fn an_invoice_the_portal_does_not_know_yet_gets_a_message_not_an_error() {
    let portal = Arc::new(Mutex::new(Portal::Empty));
    let (env, _server) = portal_env(&portal);
    let body = json(&env.get("/api/facturas/FEV9/portal", &[]));
    assert_eq!(body["consultado"], json!(false));
    assert_eq!(
        body["mensaje"],
        json!("El portal aún no tiene registro de esta factura")
    );
    assert_eq!(
        (body["origen"].clone(), body["error"].clone()),
        (json!("portal"), json!(""))
    );
    assert_eq!(body["hallazgos"], json!([]));
    assert_eq!(body["documento_soporte"], Value::Null);
    // Nothing was written: "not found yet" never creates a record.
    assert!(
        PortalStore::new(env.db.pool())
            .get("FEV9")
            .expect("get")
            .is_none()
    );
}

#[test]
fn a_non_final_record_is_queried_again_only_after_thirty_seconds() {
    let portal = Arc::new(Mutex::new(Portal::Row(portal_row("FEV1", "EN PROCESO"))));
    let (env, server) = portal_env(&portal);
    env.get("/api/facturas/FEV1/portal", &[]);
    assert_eq!(portal_calls(&server), 1);
    env.get("/api/facturas/FEV1/portal", &[]);
    assert_eq!(portal_calls(&server), 1, "younger than 30 s");

    let mut conn = env.db.conn();
    diesel::RunQueryDsl::execute(
        diesel::sql_query("UPDATE portal_cargue SET consultado_en = now() - interval '31 seconds'"),
        &mut conn,
    )
    .expect("age the record");
    drop(conn);
    *portal.lock().expect("portal") = Portal::Row(portal_row("FEV1", "VALIDADO"));
    let body = json(&env.get("/api/facturas/FEV1/portal", &[]));
    assert_eq!(
        (
            body["estado"].clone(),
            body["origen"].clone(),
            body["consultas"].clone()
        ),
        (json!("VALIDADO"), json!("portal"), json!(2))
    );
    assert_eq!(portal_calls(&server), 2);
}

#[test]
fn without_portal_settings_the_lookup_says_so_and_still_answers_200() {
    let env = env();
    let body = json(&env.get("/api/facturas/FEV1/portal", &[]));
    assert_eq!(body["error"], json!("Portal de Mutual Ser no configurado"));
    assert_eq!(
        (body["consultado"].clone(), body["origen"].clone()),
        (json!(false), json!(""))
    );
    let posted = env.post("/api/facturas/FEV1/portal/actualizar");
    assert_eq!(posted.status, 200);
    assert_eq!(
        json(&posted)["error"],
        json!("Portal de Mutual Ser no configurado")
    );
}

#[test]
fn the_portal_routes_validate_the_invoice_and_the_method() {
    let env = env();
    for bad in ["12345", "AB1", "FEV", "FEV-1x", "%20"] {
        let response = env.get(&format!("/api/facturas/{bad}/portal"), &[]);
        assert_eq!(
            (response.status, error(&response)),
            (400, "factura inválida".to_owned()),
            "{bad}"
        );
        let response = env.post(&format!("/api/facturas/{bad}/portal/actualizar"));
        assert_eq!(response.status, 400, "{bad}");
    }
    assert_eq!(env.post("/api/facturas/FEV1/portal").status, 405);
    assert_eq!(
        env.get("/api/facturas/FEV1/portal/actualizar", &[]).status,
        405
    );
}
