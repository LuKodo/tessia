//! The non-read routes: health, nota técnica, parameters, manual upload.

#[path = "api_support.rs"]
mod support;

use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use support::*;
use tessia::api::Request;
use tessia::nt::{MemoryNt, NtCatalog, NtError, NtResult, NtRow};
use tessia::storage::{ListPage, ObjectStore, StorageError, StorageResult};

fn listener() -> (TcpListener, String) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}", l.local_addr().expect("addr"));
    (l, url)
}

fn closed_url() -> String {
    let (l, url) = listener();
    drop(l);
    url
}

// --- health -------------------------------------------------------------------

#[test]
fn health_lists_the_services_in_order_with_their_state() {
    let (_medicar, medicar_url) = listener();
    let (_zona_login, zona_login) = listener();
    let (_zona_api, zona_api) = listener();
    let sispro = closed_url();
    let h = Harness::with_settings(|s| {
        s.with("medicar.base_url", &medicar_url)
            .with("minsalud.base_url", &sispro)
            .with("zonaser.login_url", &zona_login)
            .with("zonaser.benavente_base_url", &zona_api)
    });
    let resp = h.get("/api/health");
    assert_eq!(resp.status, 200);
    assert_eq!(
        resp.header("Content-Type"),
        Some("application/json; charset=utf-8")
    );
    let rows = json(&resp);
    let rows = rows.as_array().expect("array");
    let names: Vec<&str> = rows
        .iter()
        .map(|r| r["nombre"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        names,
        [
            "Medicar",
            "SISPRO",
            "ZonaSer · radicación",
            "PortalMutualser · autorizaciones",
            "MySQL",
            "Redis · nota técnica",
            "Almacenamiento · cargues",
        ]
    );
    assert_eq!(rows[0]["disponible"], true);
    assert!(rows[0].get("detalle").is_none(), "no detail when available");
    assert!(rows[0]["latencia_ms"].is_u64());
    assert_eq!(rows[1]["disponible"], false);
    assert!(!rows[1]["detalle"].as_str().unwrap_or("").is_empty());
    assert_eq!(rows[2]["disponible"], true, "both ZonaSer URLs answer");
    assert_eq!(rows[3]["disponible"], false);
    assert_eq!(rows[3]["detalle"], "no configurado");
    assert_eq!(rows[3]["latencia_ms"], 0);
    for row in &rows[4..] {
        assert_eq!(row["disponible"], true, "{row}");
    }
}

#[test]
fn health_reports_a_group_as_down_when_one_url_is_missing_or_invalid() {
    let (_login, login_url) = listener();
    let h = Harness::with_settings(|s| {
        s.with("zonaser.login_url", &login_url)
            .with("medicar.base_url", "not a url")
    });
    let rows = json(&h.get("/api/health"));
    assert_eq!(rows[0]["detalle"], "URL invalida");
    assert_eq!(rows[2]["disponible"], false);
    assert_eq!(rows[2]["detalle"], "una o más URL no disponibles");
}

struct BrokenNt;
impl NtCatalog for BrokenNt {
    fn current_version(&self) -> NtResult<Option<String>> {
        Err(NtError::Redis(
            "connection refused: redis://user:secret@h:1".into(),
        ))
    }
    fn versions(&self) -> NtResult<Vec<String>> {
        Err(NtError::Redis("x".into()))
    }
    fn meta(&self, _: &str) -> NtResult<std::collections::BTreeMap<String, String>> {
        Err(NtError::Redis("x".into()))
    }
    fn lookup(&self, _: &str, _: &str, _: &str) -> NtResult<Vec<NtRow>> {
        Err(NtError::Redis("x".into()))
    }
    fn row(&self, _: &str, _: &str) -> NtResult<Option<NtRow>> {
        Err(NtError::Redis("x".into()))
    }
}

struct BrokenBucket;
impl ObjectStore for BrokenBucket {
    fn put_bytes(&self, _: &str, _: &[u8], _: &str) -> StorageResult<()> {
        Err(StorageError::Failed("down".into()))
    }
    fn put_file(&self, _: &str, _: &Path, _: &str) -> StorageResult<()> {
        Err(StorageError::Failed("down".into()))
    }
    fn get_bytes(&self, _: &str) -> StorageResult<Vec<u8>> {
        Err(StorageError::Failed("down".into()))
    }
    fn get_to_file(&self, _: &str, _: &Path) -> StorageResult<u64> {
        Err(StorageError::Failed("down".into()))
    }
    fn exists(&self, _: &str) -> StorageResult<bool> {
        Err(StorageError::Failed("https://host/secret-token".into()))
    }
    fn delete(&self, _: &str) -> StorageResult<bool> {
        Err(StorageError::Failed("down".into()))
    }
    fn list(&self, _: Option<&str>, _: usize) -> StorageResult<ListPage> {
        Err(StorageError::Failed("down".into()))
    }
}

#[test]
fn health_rows_for_redis_and_the_bucket_never_echo_driver_errors() {
    let h = Harness::with_context(|ctx| {
        ctx.with_nt(Arc::new(BrokenNt))
            .with_objects(Arc::new(BrokenBucket))
    });
    let resp = h.get("/api/health");
    let body = String::from_utf8_lossy(&resp.body).into_owned();
    assert!(!body.contains("secret"), "{body}");
    let rows = json(&resp);
    assert_eq!(rows[5]["nombre"], "Redis · nota técnica");
    assert_eq!(rows[5]["disponible"], false);
    assert_eq!(rows[5]["detalle"], "no se pudo consultar Redis");
    assert_eq!(rows[6]["disponible"], false);
    assert_eq!(rows[6]["detalle"], "no se pudo consultar el almacenamiento");
    assert_eq!(rows[4]["disponible"], true, "the database is independent");
}

// --- nota técnica ---------------------------------------------------------------

#[test]
fn nota_tecnica_without_a_collection() {
    let h = Harness::new();
    let resp = h.get("/api/nota-tecnica");
    assert_eq!(resp.status, 200);
    assert_eq!(
        json(&resp),
        json!({"exists": false, "file_name": "", "version": "", "filas": ""})
    );
}

#[test]
fn nota_tecnica_reports_the_current_version_and_its_row_count() {
    let nt = MemoryNt::new()
        .with_version("2026-09", vec![])
        .with_version("2026-10", vec![("1".into(), NtRow::default())])
        .with_meta("2026-10", &[("filas", "1234"), ("fuente", "prueba")]);
    let h = Harness::with_nt(nt);
    assert_eq!(
        json(&h.get("/api/nota-tecnica")),
        json!({"exists": true, "file_name": "2026-10", "version": "2026-10", "filas": "1234"})
    );
}

#[test]
fn nota_tecnica_without_filas_metadata_and_without_an_upload_route() {
    let h = Harness::with_nt(MemoryNt::new().with_version("v1", vec![]));
    assert_eq!(json(&h.get("/api/nota-tecnica"))["filas"], "");
    let resp = h.send(Request::new("POST", "/api/nota-tecnica"));
    assert_eq!(resp.status, 405);
    assert_eq!(resp.header("Allow"), Some("GET"));
}

#[test]
fn nota_tecnica_failure_is_a_generic_503() {
    let h = Harness::with_context(|ctx| ctx.with_nt(Arc::new(BrokenNt)));
    let resp = h.get("/api/nota-tecnica");
    assert_eq!(resp.status, 503);
    assert_eq!(error_of(&resp), "no se pudo consultar la nota técnica");
    assert!(!String::from_utf8_lossy(&resp.body).contains("secret"));
}

// --- parameters -------------------------------------------------------------------

fn put_params(h: &Harness, body: &str) -> tessia::api::Response {
    h.send(Request::new("PUT", "/api/parameters").with_body(body))
}

const VALID: &str = r#"{"max_invoice_amount":"250000.50","automatic_subplans":["Evento POS","Otro Plan"],"wave_size":10,"derechos_unavailable_percent":40,"derechos_min_sample":5,"cuv_token_failure_percent":30,"cuv_min_sample":6}"#;

#[test]
fn parameters_default_to_configured_false_with_the_configured_cap() {
    let h = Harness::with_settings(|s| s.with("convenio.max_invoice_amount", " 1500000 "));
    let resp = h.get("/api/parameters");
    assert_eq!(resp.status, 200);
    assert_eq!(
        json(&resp),
        json!({
            "configured": false,
            "max_invoice_amount": "1500000",
            "automatic_subplans": ["EVENTO POS", "EVENTO NO PBS", "TRANSFERENCIA DE PENDIENTES CON AUTORIZACION"],
            "wave_size": 20,
            "derechos_unavailable_percent": 50,
            "derechos_min_sample": 4,
            "cuv_token_failure_percent": 50,
            "cuv_min_sample": 4
        })
    );
    let blank = Harness::new();
    assert_eq!(
        json(&blank.get("/api/parameters"))["max_invoice_amount"],
        ""
    );
}

#[test]
fn put_saves_and_get_returns_what_was_saved() {
    let h = Harness::new();
    let resp = put_params(&h, VALID);
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    let expected = json!({
        "configured": true,
        "max_invoice_amount": "250000.5",
        "automatic_subplans": ["Evento POS", "Otro Plan"],
        "wave_size": 10,
        "derechos_unavailable_percent": 40,
        "derechos_min_sample": 5,
        "cuv_token_failure_percent": 30,
        "cuv_min_sample": 6
    });
    assert_eq!(json(&resp), expected);
    assert_eq!(json(&h.get("/api/parameters")), expected);
    let stored = h.state.ctx.parametros().get().expect("get").expect("row");
    assert_eq!(stored.max_invoice_amount.to_string(), "250000.50");
    assert_eq!(stored.automatic_subplans, ["Evento POS", "Otro Plan"]);
}

#[test]
fn put_accepts_capitalized_keys_like_go_and_a_second_put_replaces() {
    let h = Harness::new();
    assert_eq!(
        put_params(&h, &VALID.replace("wave_size", "Wave_Size")).status,
        200
    );
    let again = VALID.replace("\"wave_size\":10", "\"wave_size\":0");
    assert_eq!(put_params(&h, &again).status, 200);
    assert_eq!(json(&h.get("/api/parameters"))["wave_size"], 0);
}

#[test]
fn put_rejects_unknown_fields_trailing_json_and_bad_bodies_with_400() {
    let h = Harness::new();
    let unknown = VALID.replace("{\"max", "{\"extra\":1,\"max");
    let trailing = format!("{VALID}{VALID}");
    for body in [
        unknown.as_str(),
        trailing.as_str(),
        "",
        "[]",
        "{nope",
        r#"{"wave_size":"x"}"#,
    ] {
        let resp = put_params(&h, body);
        assert_eq!(resp.status, 400, "{body}");
        assert!(
            error_of(&resp).starts_with("cuerpo invalido: "),
            "{body}: {}",
            error_of(&resp)
        );
    }
    assert!(
        h.state.ctx.parametros().get().expect("get").is_none(),
        "nothing saved"
    );
}

#[test]
fn put_validates_with_the_spanish_messages_of_the_store() {
    let h = Harness::new();
    let cases = [
        (
            VALID.replace("250000.50", "1.234,5"),
            "monto máximo inválido: use pesos sin separador de miles y máximo dos decimales, p. ej. 100000",
        ),
        (
            VALID.replace("250000.50", "0"),
            "el monto máximo debe ser positivo",
        ),
        (
            VALID.replace("250000.50", ""),
            "monto máximo inválido: use pesos sin separador de miles y máximo dos decimales, p. ej. 100000",
        ),
        (
            VALID.replace("\"wave_size\":10", "\"wave_size\":501"),
            "el tamaño de oleada debe estar entre 0 (desactivado) y 500",
        ),
        (
            VALID.replace("\"wave_size\":10", "\"wave_size\":99999999999"),
            "el tamaño de oleada debe estar entre 0 (desactivado) y 500",
        ),
        (
            VALID.replace("\"cuv_min_sample\":6", "\"cuv_min_sample\":-1"),
            "la muestra mínima de CUV debe estar entre 0 y 1000",
        ),
        (
            VALID.replace("[\"Evento POS\",\"Otro Plan\"]", "[]"),
            "se requiere al menos un subplan automático",
        ),
        (
            VALID.replace("\"Otro Plan\"", "\"  \""),
            "los subplanes automáticos no pueden estar en blanco",
        ),
        (
            VALID.replace("\"Otro Plan\"", "\" evento  pos \""),
            "subplan automático duplicado «evento  pos»",
        ),
        (
            r#"{"max_invoice_amount":"100"}"#.to_owned(),
            "se requiere al menos un subplan automático",
        ),
    ];
    for (body, message) in cases {
        let resp = put_params(&h, &body);
        assert_eq!(resp.status, 400, "{body}");
        assert_eq!(error_of(&resp), message, "{body}");
    }
    assert!(h.state.ctx.parametros().get().expect("get").is_none());
}

#[test]
fn put_from_another_origin_saves_nothing() {
    let h = Harness::new();
    let resp = h.send(
        Request::new("PUT", "/api/parameters")
            .with_header("Origin", "https://evil.example")
            .with_body(VALID),
    );
    assert_eq!(resp.status, 403);
    assert!(h.state.ctx.parametros().get().expect("get").is_none());
}

#[test]
fn put_over_the_json_limit_is_413() {
    let h = Harness::new();
    let big = format!(r#"{{"automatic_subplans":["{}"]}}"#, "x".repeat(70 << 10));
    let resp = put_params(&h, &big);
    assert_eq!(
        (resp.status, error_of(&resp).as_str()),
        (413, "cuerpo demasiado grande")
    );
}
