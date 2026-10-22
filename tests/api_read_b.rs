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
