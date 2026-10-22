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
