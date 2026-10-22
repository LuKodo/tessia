//! The Factory HKA client (port of Go `internal/adapters/thefactory`).
//!
//! ```text
//! POST {base}/api/archivos/GenerarContenedor  -> "ContenedorXml" (Base64 DIAN AttachedDocument)
//! POST {base}/api/archivos/DescargaPDF        -> "documento"     (Base64 PDF)
//! ```
//!
//! Body: `{"tokenEmpresa","tokenPassword","documento": <invoice>}`. Classification
//! and retries follow the Java code (TF-03, TF-04): up to 3 attempts, retrying
//! only 408/429/5xx (HTTP status or the `codigo` member) and an empty 200 body,
//! pausing 700 ms + 350 ms x attempt + 100-450 ms jitter *without* holding a
//! limiter slot. `204`/`404`, `codigo` 204/404 or a `mensaje` such as
//! `NO DISPONIBLE` / `NO ENCONTRADO` / `NO EXISTE` / `NO HA VIAJADO` /
//! `DIAN`...`NO`|`PENDIENT` mean the document is not available. Network errors
//! and unparseable bodies are not retried.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use base64::Engine;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use rand::Rng;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use url::Url;

use crate::http::AdaptiveLimiter;
use crate::shutdown;

/// Which document to download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The AttachedDocument container (`GenerarContenedor`).
    Xml,
    /// The invoice representation (`DescargaPDF`).
    Pdf,
}

impl Kind {
    /// `XML` or `PDF`, as printed in the status lines.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Xml => "XML",
            Kind::Pdf => "PDF",
        }
    }

    fn endpoint(self) -> (&'static str, &'static str) {
        match self {
            Kind::Pdf => ("/api/archivos/DescargaPDF", "documento"),
            Kind::Xml => ("/api/archivos/GenerarContenedor", "ContenedorXml"),
        }
    }
}

/// The Java result code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Downloaded,
    NotAvailable,
    Failed,
}

impl Status {
    /// `DESCARGADO`, `NO_DISPONIBLE` or `ERROR`.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Downloaded => "DESCARGADO",
            Status::NotAvailable => "NO_DISPONIBLE",
            Status::Failed => "ERROR",
        }
    }
}

/// One download.
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub status: Status,
    /// The decoded bytes when `status` is [`Status::Downloaded`].
    pub data: Vec<u8>,
    /// Explains `NotAvailable`/`Failed` (never contains credentials).
    pub detail: String,
}

impl FetchResult {
    fn failed(detail: String) -> Self {
        Self {
            status: Status::Failed,
            data: Vec::new(),
            detail,
        }
    }

    fn not_available(detail: String) -> Self {
        Self {
            status: Status::NotAvailable,
            data: Vec::new(),
            detail,
        }
    }
}

/// Bounds a response body (Base64 PDFs can be a few MB).
const MAX_RESPONSE: u64 = 64 << 20;
const MAX_ATTEMPTS: u32 = 3;

/// Pause (callback so tests do not sleep) between attempts.
pub type Sleeper = Arc<dyn Fn(Duration) + Send + Sync>;

/// Everything a [`TheFactoryClient`] needs.
pub struct ClientConfig {
    /// `scheme://host[:port]` (`thefactory.base_url`).
    pub base_url: String,
    pub token_empresa: String,
    pub token_password: String,
    /// The blocking client (60 s timeout; the timer starts when the request is
    /// sent, that is after the limiter slot is granted).
    pub http: Client,
    /// Paces every exchange; the slot is held while the body is read and is not
    /// held while sleeping between attempts.
    pub limiter: Option<Arc<AdaptiveLimiter>>,
    /// Replaces the real pause between attempts (tests).
    pub sleeper: Option<Sleeper>,
}

/// Talks to The Factory.
pub struct TheFactoryClient {
    base: Url,
    token_empresa: String,
    token_password: String,
    http: Client,
    limiter: Option<Arc<AdaptiveLimiter>>,
    sleeper: Option<Sleeper>,
}

impl TheFactoryClient {
    /// Validates the configuration.
    pub fn new(config: ClientConfig) -> Result<Self> {
        let base = parse_base(&config.base_url)?;
        if config.token_empresa.is_empty() || config.token_password.is_empty() {
            bail!(
                "thefactory: thefactory.token_empresa y thefactory.token_password son obligatorios"
            );
        }
        Ok(Self {
            base,
            token_empresa: config.token_empresa,
            token_password: config.token_password,
            http: config.http,
            limiter: config.limiter,
            sleeper: config.sleeper,
        })
    }

    /// Downloads one document. `Err` only when the process is interrupted.
    pub fn fetch(&self, kind: Kind, invoice: &str) -> Result<FetchResult> {
        let (path, field) = kind.endpoint();
        let body = request_body(&self.token_empresa, &self.token_password, invoice);
        let target = format!("{}{path}", self.base.as_str().trim_end_matches('/'));
        let label = kind.label();
        for attempt in 1..=MAX_ATTEMPTS {
            if shutdown::requested() {
                bail!("interrumpido");
            }
            let (status, raw) = match self.post(&target, &body) {
                Ok(answer) => answer,
                Err(err) => {
                    let host = self.base.host_str().unwrap_or_default();
                    return Ok(FetchResult::failed(format!("{label} {host}{path}: {err}")));
                }
            };
            let obj = match parse_object(&raw) {
                Ok(obj) => obj,
                Err(()) => {
                    return Ok(FetchResult::failed(format!(
                        "{label}: respuesta no JSON (HTTP {status})"
                    )));
                }
            };
            if is_transient(status, obj.as_ref()) {
                if attempt < MAX_ATTEMPTS {
                    self.pause(attempt);
                    continue;
                }
                return Ok(FetchResult::failed(format!(
                    "{label}: HTTP {status} transitorio tras {MAX_ATTEMPTS} intentos"
                )));
            }
            return Ok(interpret(kind, field, status, obj.as_ref()));
        }
        Ok(FetchResult::failed(String::new()))
    }

    fn post(&self, target: &str, body: &str) -> Result<(u16, Vec<u8>), String> {
        let exchange = || -> Result<(u16, (u16, Vec<u8>)), String> {
            let response = self
                .http
                .post(target)
                .header("Content-Type", "application/json")
                .body(body.to_owned())
                .send()
                .map_err(|e| e.without_url().to_string())?;
            let status = response.status().as_u16();
            let mut raw = Vec::new();
            response
                .take(MAX_RESPONSE)
                .read_to_end(&mut raw)
                .map_err(|e| e.to_string())?;
            Ok((status, (status, raw)))
        };
        let (status, mut raw) = match &self.limiter {
            Some(limiter) => limiter.exchange(exchange)?,
            None => exchange()?.1,
        };
        // Java read the body line by line and joined the lines.
        raw.retain(|b| *b != b'\r' && *b != b'\n');
        Ok((status, raw))
    }

    fn pause(&self, attempt: u32) {
        let delay = pause_for(attempt);
        match &self.sleeper {
            Some(sleeper) => sleeper(delay),
            None => std::thread::sleep(delay),
        }
    }
}

fn parse_base(raw: &str) -> Result<Url> {
    let trimmed = raw.trim().trim_end_matches('/');
    let invalid =
        || anyhow::anyhow!("thefactory: thefactory.base_url debe ser scheme://host[:puerto]");
    let url = Url::parse(trimmed).map_err(|_| invalid())?;
    let ok = matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.path().trim_matches('/').is_empty()
        && url.query().is_none();
    if ok { Ok(url) } else { Err(invalid()) }
}

/// The JSON request body, compact, member order as in Go.
pub(crate) fn request_body(token_empresa: &str, token_password: &str, invoice: &str) -> String {
    let body = serde_json::json!({
        "tokenEmpresa": token_empresa,
        "tokenPassword": token_password,
        "documento": invoice,
    });
    body.to_string()
}

/// 700 ms + 350 ms x attempt + 100..=450 ms jitter.
pub(crate) fn pause_for(attempt: u32) -> Duration {
    let jitter = rand::rng().random_range(0..351u64);
    Duration::from_millis(700 + 350 * u64::from(attempt) + 100 + jitter)
}

/// `Ok(None)` for an empty body, `Ok(Some(object))` for a JSON object, `Err`
/// for anything else (a JSON `null` included, as in Go).
fn parse_object(raw: &[u8]) -> Result<Option<Map<String, Value>>, ()> {
    if raw.is_empty() {
        return Ok(None);
    }
    match serde_json::from_slice::<Value>(raw) {
        Ok(Value::Object(map)) => Ok(Some(map)),
        _ => Err(()),
    }
}

/// `getAsString` of a primitive member (`""` when absent or null).
fn text(obj: &Map<String, Value>, key: &str) -> String {
    match obj.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string().trim().to_owned(),
    }
}

/// The `codigo` member as an int, else the HTTP status.
fn codigo(status: u16, obj: &Map<String, Value>) -> i64 {
    let s = text(obj, "codigo");
    if s.is_empty() {
        return i64::from(status);
    }
    if let Ok(n) = s.parse::<i64>() {
        return n;
    }
    if let Ok(f) = s.parse::<f64>() {
        return f as i64;
    }
    i64::from(status)
}

/// 408/429/5xx (status or `codigo`), or an empty 200.
pub(crate) fn is_transient(status: u16, obj: Option<&Map<String, Value>>) -> bool {
    if status == 408 || status == 429 || status >= 500 {
        return true;
    }
    let Some(obj) = obj else {
        return status == 200;
    };
    let c = codigo(status, obj);
    c == 408 || c == 429 || c >= 500
}

/// 204/404 (status or `codigo`) or one of the Java `mensaje` phrases.
pub(crate) fn is_not_available(status: u16, obj: Option<&Map<String, Value>>) -> bool {
    if status == 204 || status == 404 {
        return true;
    }
    let Some(obj) = obj else {
        return false;
    };
    let c = codigo(status, obj);
    if c == 204 || c == 404 {
        return true;
    }
    let m = text(obj, "mensaje").to_uppercase();
    [
        "NO DISPONIBLE",
        "NO ENCONTRADO",
        "NO EXISTE",
        "NO HA VIAJADO",
    ]
    .iter()
    .any(|p| m.contains(p))
        || (m.contains("DIAN") && (m.contains("NO") || m.contains("PENDIENT")))
}

fn interpret(
    kind: Kind,
    field: &str,
    status: u16,
    obj: Option<&Map<String, Value>>,
) -> FetchResult {
    let label = kind.label();
    if is_not_available(status, obj) {
        let mensaje = obj.map(|o| text(o, "mensaje")).unwrap_or_default();
        return FetchResult::not_available(format!("{label}: HTTP {status} {mensaje}"));
    }
    let member = obj.and_then(|o| o.get(field));
    let value = match member {
        None | Some(Value::Null) => {
            return FetchResult::not_available(format!("{label}: HTTP {status} sin {field}"));
        }
        Some(value) => value,
    };
    let Value::String(b64) = value else {
        return FetchResult::failed(format!("{label}: {field} no es texto"));
    };
    if kind == Kind::Xml && b64.trim().is_empty() {
        return FetchResult::not_available(format!("{label}: {field} vacio"));
    }
    match decode_base64(b64) {
        Ok(data) => FetchResult {
            status: Status::Downloaded,
            data,
            detail: String::new(),
        },
        Err(err) => FetchResult::failed(format!("{label}: Base64 invalido: {err}")),
    }
}

/// `java.util.Base64.getDecoder()`: padding optional.
fn decode_base64(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    let engine = GeneralPurpose::new(
        &STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    engine.decode(s.trim_end_matches('='))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::testing::{MockResponse, MockServer};

    fn client_for(server: &MockServer, sleeps: Arc<Mutex<Vec<Duration>>>) -> TheFactoryClient {
        TheFactoryClient::new(ClientConfig {
            base_url: format!("{}/", server.url()),
            token_empresa: "emp".into(),
            token_password: "pw".into(),
            http: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("client"),
            limiter: None,
            sleeper: Some(Arc::new(move |d| sleeps.lock().expect("sleeps").push(d))),
        })
        .expect("client")
    }

    fn b64(data: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(data)
    }

    #[test]
    fn request_body_and_endpoints_follow_go() {
        let server = MockServer::sequence(vec![MockResponse::json(
            200,
            &format!(r#"{{"ContenedorXml":"{}"}}"#, b64(b"<a/>")),
        )]);
        let sleeps = Arc::default();
        let client = client_for(&server, sleeps);
        let xml = client.fetch(Kind::Xml, "FEV1").expect("fetch");
        assert_eq!(xml.status, Status::Downloaded);
        assert_eq!(xml.data, b"<a/>");
        let _ = client.fetch(Kind::Pdf, "FEV1").expect("fetch");
        let requests = server.requests();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/api/archivos/GenerarContenedor");
        assert_eq!(requests[1].path, "/api/archivos/DescargaPDF");
        assert_eq!(
            requests[0].body_text(),
            r#"{"tokenEmpresa":"emp","tokenPassword":"pw","documento":"FEV1"}"#
        );
        assert_eq!(requests[0].header("content-type"), Some("application/json"));
    }

    #[test]
    fn base64_without_padding_is_accepted() {
        let encoded = b64(b"abcd").trim_end_matches('=').to_owned();
        assert_eq!(decode_base64(&encoded).expect("decode"), b"abcd");
        assert_eq!(decode_base64(&b64(b"abcde")).expect("decode"), b"abcde");
        assert!(decode_base64("@@@").is_err());
    }

    fn classify(status: u16, body: &str) -> (Status, String) {
        let server = MockServer::sequence(vec![MockResponse::json(status, body)]);
        let client = client_for(&server, Arc::default());
        let r = client.fetch(Kind::Xml, "FEV1").expect("fetch");
        (r.status, r.detail)
    }

    #[test]
    fn classification_table() {
        let good = format!(r#"{{"ContenedorXml":"{}"}}"#, b64(b"<a/>"));
        let cases: Vec<(u16, &str, Status)> = vec![
            (200, &good, Status::Downloaded),
            (204, "", Status::NotAvailable),
            (404, "", Status::NotAvailable),
            (200, r#"{"codigo":404}"#, Status::NotAvailable),
            (200, r#"{"codigo":"204"}"#, Status::NotAvailable),
            (
                200,
                r#"{"mensaje":"Documento NO DISPONIBLE"}"#,
                Status::NotAvailable,
            ),
            (200, r#"{"mensaje":"no encontrado"}"#, Status::NotAvailable),
            (
                200,
                r#"{"mensaje":"El documento no existe"}"#,
                Status::NotAvailable,
            ),
            (
                200,
                r#"{"mensaje":"NO HA VIAJADO a la DIAN"}"#,
                Status::NotAvailable,
            ),
            (200, r#"{"mensaje":"DIAN pendiente"}"#, Status::NotAvailable),
            (
                200,
                r#"{"mensaje":"DIAN no responde"}"#,
                Status::NotAvailable,
            ),
            (200, r#"{"otra":1}"#, Status::NotAvailable),
            (200, r#"{"ContenedorXml":null}"#, Status::NotAvailable),
            (200, r#"{"ContenedorXml":"  "}"#, Status::NotAvailable),
            (401, r#"{"mensaje":"token invalido"}"#, Status::NotAvailable),
            (200, "no es json", Status::Failed),
            (200, "null", Status::Failed),
            (200, r#"{"ContenedorXml":"@@@"}"#, Status::Failed),
            (200, r#"{"ContenedorXml":12}"#, Status::Failed),
        ];
        for (status, body, expected) in cases {
            assert_eq!(classify(status, body).0, expected, "{status} {body}");
        }
    }

    #[test]
    fn details_match_go() {
        assert_eq!(classify(404, "").1, "XML: HTTP 404 ");
        assert_eq!(
            classify(200, r#"{"mensaje":"NO EXISTE"}"#).1,
            "XML: HTTP 200 NO EXISTE"
        );
        assert_eq!(
            classify(200, r#"{"x":1}"#).1,
            "XML: HTTP 200 sin ContenedorXml"
        );
        assert_eq!(classify(200, "html").1, "XML: respuesta no JSON (HTTP 200)");
        assert_eq!(
            classify(200, r#"{"ContenedorXml":" "}"#).1,
            "XML: ContenedorXml vacio"
        );
    }

    #[test]
    fn transient_answers_are_retried_three_times_with_pauses() {
        for (status, body) in [
            (503, "{}"),
            (429, "{}"),
            (408, "{}"),
            (200, ""),
            (200, r#"{"codigo":503}"#),
        ] {
            let server = MockServer::sequence(vec![MockResponse::json(status, body)]);
            let sleeps = Arc::default();
            let client = client_for(&server, Arc::clone(&sleeps));
            let r = client.fetch(Kind::Pdf, "FEV1").expect("fetch");
            assert_eq!(r.status, Status::Failed, "{status} {body}");
            assert_eq!(
                r.detail,
                format!("PDF: HTTP {status} transitorio tras 3 intentos")
            );
            assert_eq!(server.requests().len(), 3, "{status} {body}");
            let pauses = sleeps.lock().expect("sleeps").clone();
            assert_eq!(pauses.len(), 2, "no pause after the last attempt");
            for (i, pause) in pauses.iter().enumerate() {
                let base = 700 + 350 * (i as u64 + 1) + 100;
                let ms = pause.as_millis() as u64;
                assert!(
                    (base..=base + 350).contains(&ms),
                    "pause {ms} ms attempt {}",
                    i + 1
                );
            }
        }
    }

    #[test]
    fn a_transient_answer_followed_by_a_good_one_succeeds() {
        let good = format!(r#"{{"documento":"{}"}}"#, b64(b"%PDF-1.4"));
        let server = MockServer::sequence(vec![
            MockResponse::json(503, "{}"),
            MockResponse::json(200, &good),
        ]);
        let client = client_for(&server, Arc::default());
        let r = client.fetch(Kind::Pdf, "FEV1").expect("fetch");
        assert_eq!(r.status, Status::Downloaded);
        assert_eq!(r.data, b"%PDF-1.4");
        assert_eq!(server.requests().len(), 2);
    }

    #[test]
    fn network_errors_are_failures_without_retry() {
        let server = MockServer::sequence(vec![MockResponse::json(200, "{}")]);
        let url = server.url().to_owned();
        drop(server);
        let client = TheFactoryClient::new(ClientConfig {
            base_url: url,
            token_empresa: "e".into(),
            token_password: "p".into(),
            http: Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .expect("client"),
            limiter: None,
            sleeper: Some(Arc::new(|_| {})),
        })
        .expect("client");
        let r = client.fetch(Kind::Xml, "FEV1").expect("fetch");
        assert_eq!(r.status, Status::Failed);
        assert!(
            r.detail
                .starts_with("XML 127.0.0.1/api/archivos/GenerarContenedor: "),
            "{}",
            r.detail
        );
        assert!(!r.detail.contains("pw"));
    }

    #[test]
    fn configuration_is_validated() {
        for bad in ["", "ftp://h", "http://h/path", "http://h?x=1", "no url"] {
            assert!(parse_base(bad).is_err(), "{bad}");
        }
        assert!(parse_base("https://h.example:8443/").is_ok());
    }

    #[test]
    fn pause_bounds() {
        for attempt in 1..=2u32 {
            for _ in 0..50 {
                let ms = pause_for(attempt).as_millis() as u64;
                let base = 700 + 350 * u64::from(attempt) + 100;
                assert!((base..=base + 350).contains(&ms));
            }
        }
    }
}
