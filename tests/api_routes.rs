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
            "PostgreSQL",
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
fn nota_tecnica_failure_answers_200_with_nothing_published() {
    let h = Harness::with_context(|ctx| ctx.with_nt(Arc::new(BrokenNt)));
    let resp = h.get("/api/nota-tecnica");
    assert_eq!(resp.status, 200);
    assert_eq!(
        json(&resp),
        serde_json::json!({"exists": false, "file_name": "", "version": "", "filas": ""})
    );
    assert!(!String::from_utf8_lossy(&resp.body).contains("secret"));
}

#[test]
fn nota_tecnica_file_name_is_the_first_12_characters_of_the_version() {
    let version = "0123456789abcdef0123456789abcdef";
    let h = Harness::with_nt(MemoryNt::new().with_version(version, vec![]));
    let body = json(&h.get("/api/nota-tecnica"));
    assert_eq!(body["file_name"], "0123456789ab");
    assert_eq!(body["version"], version);
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

// --- manual upload ------------------------------------------------------------------

const HEADER: &str = "nit|plan|subplan|factura|tipo de identificacion|no. documento|no. formula|fecha|plu|articulo|cantidad|vlr. unitario|vlr. total|cuota|mipres|cum";

fn row(invoice: &str) -> String {
    format!(
        "900000001|PLAN|EVENTO POS|{invoice}|CC|1001||2026-10-21 10:00:00|20010_01|PRODUCTO|1|1500|1500|0||19950404-1"
    )
}

fn valid_csv() -> String {
    format!("{HEADER}\n{}\n{}\n", row("EVS-1"), row("EVS-2"))
}

const BOUNDARY: &str = "----tessiaTestBoundary";

fn multipart(parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, file, data) in parts {
        out.extend(
            format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"").bytes(),
        );
        if let Some(file) = file {
            out.extend(format!("; filename=\"{file}\"\r\nContent-Type: text/csv").bytes());
        }
        out.extend(b"\r\n\r\n");
        out.extend(*data);
        out.extend(b"\r\n");
    }
    out.extend(format!("--{BOUNDARY}--\r\n").bytes());
    out
}

fn upload_request(body: Vec<u8>) -> Request {
    Request::new("POST", "/api/convenio/upload")
        .with_header(
            "Content-Type",
            &format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .with_body(body)
}

fn upload_csv(h: &Harness, file_name: &str, csv: &[u8]) -> tessia::api::Response {
    h.send(upload_request(multipart(&[("csv", Some(file_name), csv)])))
}

fn cargues(h: &Harness) -> Vec<tessia::stores::cargues::Cargue> {
    let since = chrono::DateTime::from_timestamp(0, 0).expect("epoch");
    h.state.ctx.cargues().recent(since, 100).expect("recent")
}

fn assert_nothing_stored(h: &Harness) {
    assert!(cargues(h).is_empty(), "no cargue registered");
    assert!(h.objects.is_empty(), "no object published");
}

fn is_manual_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("Manual_") else {
        return false;
    };
    let Some((stamp, hash)) = rest.split_once("Z_") else {
        return false;
    };
    let (date, time) = stamp.split_once('T').unwrap_or_default();
    date.len() == 8
        && time.len() == 15
        && stamp.bytes().all(|b| b.is_ascii_digit() || b == b'T')
        && hash.len() == 12
        && hash
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[test]
fn a_valid_csv_becomes_a_manual_cargue_with_its_csv_published() {
    use sha2::{Digest, Sha256};
    let h = Harness::new();
    let csv = valid_csv();
    let resp = upload_csv(&h, "mi convenio.csv", csv.as_bytes());
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    let body = json(&resp);
    assert_eq!(body["ok"], true);
    let id = body["cargue"].as_str().expect("cargue").to_owned();
    assert!(is_manual_id(&id), "{id}");
    assert_eq!(body["file_name"], format!("{id}.csv"));
    assert_eq!(body.as_object().map(|o| o.len()), Some(3));

    let sha = hex::encode(Sha256::digest(csv.as_bytes()));
    assert!(id.ends_with(&sha[..12]));
    let stored = h
        .state
        .ctx
        .cargues()
        .newest(&id)
        .expect("newest")
        .expect("row");
    assert_eq!(stored.sha256, sha);
    assert_eq!(stored.name, format!("{id}.csv"));
    assert_eq!(stored.origin().as_str(), "manual");

    assert_eq!(h.objects.keys(), vec![format!("{id}__{id}.csv")]);
    let published = h
        .state
        .ctx
        .artifacts()
        .fetch_bytes(&id, &format!("{id}.csv"))
        .expect("fetch");
    assert_eq!(published, csv.as_bytes());
}

#[test]
fn the_same_bytes_uploaded_twice_make_two_manual_cargues() {
    let h = Harness::new();
    let first = json(&upload_csv(&h, "a.csv", valid_csv().as_bytes()));
    let second = json(&upload_csv(&h, "a.csv", valid_csv().as_bytes()));
    assert_ne!(first["cargue"], second["cargue"]);
    assert_eq!(cargues(&h).len(), 2);
}

#[test]
fn the_extension_is_checked_case_insensitively_and_paths_are_dropped() {
    let h = Harness::new();
    for name in [
        "DATOS.CSV",
        "C:\\Users\\x\\datos.csv",
        "/tmp/a/datos.Csv",
        ".csv",
    ] {
        assert_eq!(
            upload_csv(&h, name, valid_csv().as_bytes()).status,
            200,
            "{name}"
        );
    }
}

#[test]
fn bom_and_crlf_csv_files_are_accepted() {
    let h = Harness::new();
    let csv = format!("\u{feff}{}", valid_csv().replace('\n', "\r\n"));
    assert_eq!(upload_csv(&h, "a.csv", csv.as_bytes()).status, 200);
}

#[test]
fn anything_but_one_csv_file_field_is_refused() {
    let h = Harness::new();
    let csv = valid_csv();
    let data = csv.as_bytes();
    let bodies = [
        ("no parts", multipart(&[])),
        (
            "wrong field name",
            multipart(&[("file", Some("a.csv"), data)]),
        ),
        ("value, not a file", multipart(&[("csv", None, data)])),
        ("empty file name", multipart(&[("csv", Some(""), data)])),
        (
            "wrong extension",
            multipart(&[("csv", Some("a.xlsx"), data)]),
        ),
        ("no extension", multipart(&[("csv", Some("csv"), data)])),
        (
            "two csv files",
            multipart(&[("csv", Some("a.csv"), data), ("csv", Some("b.csv"), data)]),
        ),
        (
            "extra file field",
            multipart(&[("csv", Some("a.csv"), data), ("otro", Some("b.csv"), data)]),
        ),
    ];
    for (label, body) in bodies {
        let resp = h.send(upload_request(body));
        assert_eq!(resp.status, 400, "{label}");
        assert_eq!(
            error_of(&resp),
            "Selecciona un único archivo CSV.",
            "{label}"
        );
    }
    assert_nothing_stored(&h);
}

#[test]
fn plain_value_fields_next_to_the_file_are_ignored() {
    let h = Harness::new();
    let csv = valid_csv();
    let body = multipart(&[
        ("desde", None, b"2026-10-01"),
        ("csv", Some("a.csv"), csv.as_bytes()),
    ]);
    assert_eq!(h.send(upload_request(body)).status, 200);
}

#[test]
fn a_request_that_is_not_multipart_is_400() {
    let h = Harness::new();
    let cases = [
        Request::new("POST", "/api/convenio/upload").with_body(valid_csv()),
        Request::new("POST", "/api/convenio/upload")
            .with_header("Content-Type", "text/csv")
            .with_body(valid_csv()),
        Request::new("POST", "/api/convenio/upload")
            .with_header("Content-Type", "multipart/form-data")
            .with_body(valid_csv()),
        Request::new("POST", "/api/convenio/upload")
            .with_header("Content-Type", &format!("multipart/form-data; boundary={BOUNDARY}"))
            .with_body("--not the boundary\r\n"),
        // The closing delimiter is missing.
        Request::new("POST", "/api/convenio/upload")
            .with_header("Content-Type", &format!("multipart/form-data; boundary={BOUNDARY}"))
            .with_body(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"csv\"; filename=\"a.csv\"\r\n\r\ndata")),
    ];
    for req in cases {
        let resp = h.send(req);
        assert_eq!(resp.status, 400);
        assert_eq!(error_of(&resp), "Solicitud multipart inválida.");
    }
    assert_nothing_stored(&h);
}

#[test]
fn an_invalid_or_empty_csv_is_refused() {
    let h = Harness::new();
    let cases: [(&str, String); 5] = [
        ("empty file", String::new()),
        ("not a csv", "esto no es un convenio".to_owned()),
        ("missing columns", "nit|factura\n1|A\n".to_owned()),
        ("header only", format!("{HEADER}\n")),
        ("blank lines only", format!("{HEADER}\n\n\n")),
    ];
    for (label, csv) in cases {
        let resp = upload_csv(&h, "a.csv", csv.as_bytes());
        assert_eq!(resp.status, 400, "{label}");
        assert_eq!(
            error_of(&resp),
            "El CSV de convenio es inválido o no contiene registros válidos.",
            "{label}"
        );
    }
    assert_nothing_stored(&h);
}

#[test]
fn a_csv_with_a_malformed_row_is_refused_whole() {
    let h = Harness::new();
    let csv = format!(
        "{HEADER}\n{}\n900000001|PLAN|solo tres campos\n",
        row("EVS-1")
    );
    let resp = upload_csv(&h, "a.csv", csv.as_bytes());
    assert_eq!(resp.status, 400);
    assert_eq!(
        error_of(&resp),
        "El CSV contiene filas con columnas incompletas o mal formadas."
    );
    assert_nothing_stored(&h);
}

#[test]
fn files_above_32_mib_are_413_and_exactly_32_mib_is_not() {
    let h = Harness::new();
    let prefix = format!("{HEADER}\n{}\n", row("EVS-1"));
    let pad = |total: usize| {
        let mut data = prefix.clone().into_bytes();
        // Blank lines are skipped by the reader, so the CSV stays valid.
        data.resize(total, b'\n');
        data
    };
    let over = pad((32 << 20) + 1);
    let resp = upload_csv(&h, "big.csv", &over);
    assert_eq!(resp.status, 413);
    assert_eq!(error_of(&resp), "El CSV supera el límite de 32 MiB.");
    assert_nothing_stored(&h);

    let exact = pad(32 << 20);
    let resp = upload_csv(&h, "big.csv", &exact);
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
}

#[test]
fn a_body_above_the_framing_allowance_is_413_before_parsing() {
    let h = Harness::new();
    let resp = h.send(upload_request(vec![b'x'; (32 << 20) + (64 << 10) + 1]));
    assert_eq!(
        (resp.status, error_of(&resp).as_str()),
        (413, "El CSV supera el límite de 32 MiB.")
    );
}

#[test]
fn an_upload_from_another_origin_stores_nothing() {
    let h = Harness::new();
    let resp = h.send(
        upload_request(multipart(&[("csv", Some("a.csv"), valid_csv().as_bytes())]))
            .with_header("Origin", "https://evil.example"),
    );
    assert_eq!(resp.status, 403);
    assert_nothing_stored(&h);
}

#[test]
fn the_upload_route_only_accepts_post() {
    let h = Harness::new();
    let resp = h.get("/api/convenio/upload");
    assert_eq!((resp.status, resp.header("Allow")), (405, Some("POST")));
}

#[test]
fn a_failing_bucket_is_a_generic_500_and_registers_no_cargue() {
    let h = Harness::with_context(|ctx| ctx.with_objects(Arc::new(BrokenBucket)));
    let resp = upload_csv(&h, "a.csv", valid_csv().as_bytes());
    assert_eq!(resp.status, 500);
    assert_eq!(error_of(&resp), "No se pudo guardar el CSV.");
    assert!(!String::from_utf8_lossy(&resp.body).contains("down"));
    assert!(cargues(&h).is_empty());
}
