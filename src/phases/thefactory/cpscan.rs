//! CPScan support lookup (port of Go `internal/adapters/cpscan` and
//! `internal/soporte/acquire.go`).
//!
//! For an invoice whose SSC is known the phase asks
//! `GET {cpscan.base_url}/documentos/{ssc}` and downloads the file the document
//! points at. Decision rule ([`candidates`]):
//!
//! * the document's `adicional` holds a URL (for `PC`: any value): that is the
//!   only file tried, whatever the `origen`;
//! * otherwise the `origen` decides: `PC` has no file without `adicional`,
//!   `APP` / `BUCKET` / empty / unknown / no document use `formula-<ssc>.jpg`,
//!   `SL` tries `firma-<ssc>.jpg`, `firma-<ssc>.png` and then the formula;
//! * the kind of a file comes from its bytes ([`sniff`]): `%PDF` (>= 100 bytes),
//!   PNG, JPEG, GIF, BMP or TIFF; anything else (an error page, a truncated
//!   body) is not a usable file and the next candidate is tried;
//! * references that are not `http(s)` URLs or bucket file names are rejected
//!   (a definitive "no file", not a transient failure).
//!
//! Both calls share one limiter (6/24/35) and one client (30 s timeout,
//! `cpscan.ca_file`, `cpscan.tls_server_name`). A transient failure (network,
//! 5xx after 3 attempts, malformed JSON) is an `Err` and records nothing.

use std::collections::HashMap;
use std::io::Read;
use std::net::ToSocketAddrs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::http::{AdaptiveLimiter, ClientOptions, RetryPolicy, build_client, safe_url};
use crate::shutdown;

/// Sidecar format version written and required.
pub const SIDECAR_VERSION: u32 = 1;
/// A download smaller than this is "ARCHIVO_VACIO" (`archivoValido(.., 10)`).
const MIN_FILE_SIZE: usize = 10;
/// Guard against runaway bodies.
const MAX_FILE_SIZE: u64 = 50 << 20;
/// A PDF smaller than this is not a support (`archivoValido(pdf, 100)`).
const MIN_PDF_SIZE: usize = 100;
/// Bounds the JSON of one document.
const MAX_DOCUMENT_BODY: u64 = 1 << 20;
/// Download timeout of the CPScan client.
pub const CPSCAN_TIMEOUT: Duration = Duration::from_secs(30);
/// The old CPScan API host path; its files now live in the bucket.
const LEGACY_FILE_VIEW: &str = "api-mobile.pharmaser.com.co/file/view/";

const KNOWN_EXTS: [&str; 6] = ["pdf", "jpg", "png", "gif", "bmp", "tif"];

/// `CPSCAN/<F>.cpscan.json`: the outcome of one completed CPScan lookup.
///
/// The JSON object (written with 2-space indentation) has these members:
///
/// | member | meaning |
/// |---|---|
/// | `version` | always `1`; readers must reject other values |
/// | `ssc` | the SSC that was asked for |
/// | `documento` | `true` when the API returned a document for the SSC |
/// | `origen` | `origen` of the document (`APP`, `BUCKET`, `PC`, `SL`, ...), `""` without document |
/// | `adicional` | trimmed `adicional` of the document (a URL of the file), `""` if none |
/// | `estado` | `estado` of the document |
/// | `archivo` | the bucket file name or URL that was downloaded; `""` = CPScan has nothing |
/// | `ext` | kind of the payload: `pdf`, `jpg`, `png`, `gif`, `bmp` or `tif` (required when `archivo` is set) |
/// | `consultado` | UTC time of the lookup, RFC 3339 with seconds (omitted when unknown) |
///
/// The payload is the sibling artifact `CPSCAN/<F>.support.<ext>` (only when
/// `archivo` is not empty). The sidecar is published last, so a run interrupted
/// between both leaves the part pending. A sidecar that is missing, does not
/// parse, has another `version`, or whose payload is missing or empty means the
/// lookup is *not done* and `thefactory descargar` repeats it. Deleting the
/// sidecar forces a new lookup.
///
/// ```json
/// {
///   "version": 1,
///   "ssc": "123456",
///   "documento": true,
///   "origen": "APP",
///   "adicional": "",
///   "estado": "OK",
///   "archivo": "formula-123456.jpg",
///   "ext": "jpg",
///   "consultado": "2026-10-23T14:05:09Z"
/// }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpscanSidecar {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub ssc: String,
    /// `documento`: the API returned a document for the SSC.
    #[serde(default, rename = "documento")]
    pub found: bool,
    #[serde(default, rename = "origen")]
    pub origin: String,
    #[serde(default, rename = "adicional")]
    pub additional: String,
    #[serde(default, rename = "estado")]
    pub status: String,
    /// `archivo`: what was downloaded (`""` = nothing).
    #[serde(default, rename = "archivo")]
    pub file: String,
    #[serde(default)]
    pub ext: String,
    /// `consultado`.
    #[serde(
        default,
        rename = "consultado",
        skip_serializing_if = "String::is_empty"
    )]
    pub fetched_at: String,
}

impl CpscanSidecar {
    /// A support file was downloaded.
    pub fn has_file(&self) -> bool {
        !self.file.is_empty()
    }

    /// Reads a sidecar and rejects what a reader cannot rely on.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let sidecar: CpscanSidecar = serde_json::from_slice(bytes).context("sidecar invalido")?;
        if sidecar.version != SIDECAR_VERSION {
            bail!("sidecar invalido: version {} no soportada", sidecar.version);
        }
        if sidecar.has_file() && !KNOWN_EXTS.contains(&sidecar.ext.as_str()) {
            bail!("sidecar invalido: ext {:?}", sidecar.ext);
        }
        Ok(sidecar)
    }

    /// The sidecar JSON.
    pub fn to_json(&self) -> Result<Vec<u8>> {
        let mut sidecar = self.clone();
        sidecar.version = SIDECAR_VERSION;
        Ok(serde_json::to_vec_pretty(&sidecar)?)
    }

    /// `CPSCAN/<F>.cpscan.json`.
    pub fn sidecar_path(invoice: &str) -> String {
        format!("CPSCAN/{invoice}.cpscan.json")
    }

    /// `CPSCAN/<F>.support.<ext>`.
    pub fn payload_path(invoice: &str, ext: &str) -> String {
        format!("CPSCAN/{invoice}.support.{ext}")
    }

    /// The reason recorded when CPScan answered but has no support:
    /// `CPSCAN sin soporte (SSC <ssc>, <origen X | sin documento>)`.
    pub fn no_support_warning(&self) -> Option<String> {
        if self.has_file() {
            return None;
        }
        let origin = if self.found {
            format!("origen {}", self.origin.trim().to_uppercase())
        } else {
            "sin documento".to_owned()
        };
        Some(format!("CPSCAN sin soporte (SSC {}, {origin})", self.ssc))
    }
}

/// The CPScan document of an SSC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub ssc: String,
    pub origin: String,
    pub additional: String,
    pub status: String,
}

impl Document {
    /// Upper-case trimmed `origen`, `NULL` when empty.
    fn origin_key(&self) -> String {
        let origin = self.origin.trim().to_uppercase();
        if origin.is_empty() {
            "NULL".to_owned()
        } else {
            origin
        }
    }
}

/// A failed file download: `NoFile` is a definitive answer (try the next
/// candidate), `Failed` is transient (the lookup records nothing).
#[derive(Debug, Error)]
pub enum FileError {
    #[error("{0}")]
    NoFile(String),
    #[error("{0}")]
    Failed(String),
}

/// One HTTP client for both CPScan calls, honoring `cpscan.tls_server_name`.
///
/// With a server name and an `https` URL the connection goes to the URL's host
/// while the certificate is verified against the server name: the request URL
/// is rewritten to the server name, which is pinned to the real host's
/// addresses, and the `Host` header keeps the original authority.
pub struct CpscanHttp {
    default: Client,
    ca_file: Option<String>,
    tls_server_name: Option<String>,
    timeout: Duration,
    by_origin: Mutex<HashMap<String, Client>>,
}

/// A request ready to send.
struct Prepared {
    client: Client,
    url: String,
    host_header: Option<String>,
}

impl CpscanHttp {
    /// The CPScan client of the settings (`cpscan.ca_file`,
    /// `cpscan.tls_server_name`), 30 s timeout.
    pub fn new(ca_file: Option<&str>, tls_server_name: Option<&str>) -> Result<Self> {
        let ca_file = ca_file.filter(|v| !v.is_empty()).map(str::to_owned);
        let default = build_client(
            &ClientOptions::default()
                .with_timeout(CPSCAN_TIMEOUT)
                .with_ca_file(ca_file.as_deref()),
        )?;
        Ok(Self {
            default,
            ca_file,
            tls_server_name: tls_server_name
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned),
            timeout: CPSCAN_TIMEOUT,
            by_origin: Mutex::new(HashMap::new()),
        })
    }

    /// A plain client (tests).
    pub fn plain(client: Client) -> Self {
        Self {
            default: client,
            ca_file: None,
            tls_server_name: None,
            timeout: CPSCAN_TIMEOUT,
            by_origin: Mutex::new(HashMap::new()),
        }
    }

    fn prepare(&self, raw: &str) -> Result<Prepared> {
        let Some(name) = &self.tls_server_name else {
            return Ok(self.plain_request(raw));
        };
        let url = Url::parse(raw).with_context(|| format!("URL invalida: {raw}"))?;
        if url.scheme() != "https" {
            return Ok(self.plain_request(raw));
        }
        let (rewritten, authority) = rewrite_for_server_name(&url, name)?;
        let host = url.host_str().unwrap_or_default().to_owned();
        let port = url.port_or_known_default().unwrap_or(443);
        let client = self.client_for_origin(&format!("{host}:{port}"), &host, port, name)?;
        Ok(Prepared {
            client,
            url: rewritten,
            host_header: Some(authority),
        })
    }

    fn plain_request(&self, raw: &str) -> Prepared {
        Prepared {
            client: self.default.clone(),
            url: raw.to_owned(),
            host_header: None,
        }
    }

    fn client_for_origin(&self, key: &str, host: &str, port: u16, name: &str) -> Result<Client> {
        let mut cache = self
            .by_origin
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(client) = cache.get(key) {
            return Ok(client.clone());
        }
        let addrs: Vec<_> = (host, port)
            .to_socket_addrs()
            .with_context(|| format!("no se pudo resolver {host}"))?
            .collect();
        let mut builder = Client::builder()
            .timeout(self.timeout)
            .connect_timeout(Duration::from_secs(10))
            .resolve_to_addrs(name, &addrs);
        if let Some(path) = &self.ca_file {
            let pem = std::fs::read(path).with_context(|| format!("no se pudo leer {path}"))?;
            builder = builder.tls_built_in_root_certs(false);
            for cert in reqwest::Certificate::from_pem_bundle(&pem)? {
                builder = builder.add_root_certificate(cert);
            }
        }
        let client = builder
            .build()
            .context("no se pudo crear el cliente HTTP")?;
        cache.insert(key.to_owned(), client.clone());
        Ok(client)
    }
}

/// `url` with its host replaced by `server_name` (port kept), and the original
/// `host[:port]` for the `Host` header.
pub(crate) fn rewrite_for_server_name(url: &Url, server_name: &str) -> Result<(String, String)> {
    let host = url.host_str().ok_or_else(|| anyhow!("URL sin host"))?;
    let authority = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let mut rewritten = url.clone();
    rewritten
        .set_host(Some(server_name))
        .map_err(|e| anyhow!("tls_server_name invalido: {e}"))?;
    Ok((rewritten.to_string(), authority))
}

/// GET with the shared limiter and the Go retry rule (network errors, 408, 425,
/// 429 and 5xx): a slot is taken per attempt, kept until the body is read and
/// never held while sleeping between attempts.
pub struct CpscanTransport {
    http: CpscanHttp,
    limiter: Option<Arc<AdaptiveLimiter>>,
    retry: RetryPolicy,
}

impl CpscanTransport {
    pub fn new(
        http: CpscanHttp,
        limiter: Option<Arc<AdaptiveLimiter>>,
        retry: RetryPolicy,
    ) -> Self {
        Self {
            http,
            limiter,
            retry,
        }
    }

    /// `(status, body)`; the body holds at most `max_body + 1` bytes.
    pub fn get(&self, url: &str, accept: Option<&str>, max_body: u64) -> Result<(u16, Vec<u8>)> {
        let attempts = self.retry.max_attempts.max(1);
        let shown = Url::parse(url)
            .map(|u| safe_url(&u))
            .unwrap_or_else(|_| "<url>".to_owned());
        for attempt in 1..=attempts {
            if shutdown::requested() {
                bail!("interrumpido");
            }
            let permit = self.limiter.as_deref().map(AdaptiveLimiter::acquire);
            let outcome = self.once(url, accept, max_body);
            match outcome {
                Err(err) => {
                    if let Some(permit) = permit {
                        permit.finish(false);
                    }
                    if attempt >= attempts {
                        bail!("GET {shown}: se rindio tras {attempt} intentos: {err:#}");
                    }
                    self.sleep(self.retry.backoff(attempt));
                }
                Ok((status, body, retry_after)) => {
                    if let Some(permit) = permit {
                        permit.finish_status(status);
                    }
                    if !(retryable(status) && attempt < attempts) {
                        return Ok((status, body));
                    }
                    let delay = self.retry.backoff(attempt).max(retry_after);
                    self.sleep(delay.min(self.retry.max_delay));
                }
            }
        }
        bail!("GET {shown}: sin respuesta")
    }

    fn once(
        &self,
        url: &str,
        accept: Option<&str>,
        max_body: u64,
    ) -> Result<(u16, Vec<u8>, Duration)> {
        let prepared = self.http.prepare(url)?;
        let mut request = prepared.client.get(&prepared.url);
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }
        if let Some(host) = &prepared.host_header {
            request = request.header("Host", host);
        }
        let response = request.send().map_err(|e| anyhow!("{}", e.without_url()))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map_or(Duration::ZERO, Duration::from_secs);
        let mut body = Vec::new();
        response
            .take(max_body + 1)
            .read_to_end(&mut body)
            .map_err(|e| anyhow!("{e}"))?;
        Ok((status, body, retry_after))
    }

    fn sleep(&self, delay: Duration) {
        match &self.retry.sleeper {
            Some(sleeper) => sleeper(delay),
            None => std::thread::sleep(delay),
        }
    }
}

fn retryable(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500..=599)
}

/// `url.PathEscape`: everything but unreserved characters and `$&+=:@`.
pub(crate) fn path_escape(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'~' | b'$' | b'&' | b'+' | b'=' | b':' | b'@'
            );
        if keep {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A JSON scalar the API may send as a string, a number or null.
fn scalar_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.trim().to_owned(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Resolves the CPScan document of an SSC.
pub struct CpscanApi {
    base: Url,
    transport: Arc<CpscanTransport>,
}

impl CpscanApi {
    pub fn new(base_url: &str, transport: Arc<CpscanTransport>) -> Result<Self> {
        let base = Url::parse(base_url.trim())
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
            .ok_or_else(|| anyhow!("cpscan: cpscan.base_url invalida"))?;
        Ok(Self { base, transport })
    }

    /// `<base>/documentos/<ssc>` with the SSC escaped as one path segment, so a
    /// value such as `a/b?x` can never address another resource.
    pub fn document_url(&self, ssc: &str) -> String {
        let mut url = self.base.clone();
        url.set_query(None);
        url.set_fragment(None);
        let root = url.path().trim_end_matches('/').to_owned();
        url.set_path(&format!("{root}/documentos/{}", path_escape(ssc)));
        url.to_string()
    }

    /// The document CPScan holds for `ssc`, or `None` when it has none (404,
    /// 204 or a JSON `null`). Network errors, 5xx after the retries, any other
    /// status and a body that is not a JSON object are errors: the lookup did
    /// not complete.
    pub fn document(&self, ssc: &str) -> Result<Option<Document>> {
        let ssc = ssc.trim();
        if ssc.is_empty() {
            bail!("cpscan: SSC vacio");
        }
        let (status, body) = self
            .transport
            .get(
                &self.document_url(ssc),
                Some("application/json"),
                MAX_DOCUMENT_BODY,
            )
            .map_err(|e| anyhow!("cpscan: documento {ssc}: {e:#}"))?;
        match status {
            200 => {}
            404 | 204 => return Ok(None),
            other => bail!("cpscan: documento {ssc}: HTTP {other}"),
        }
        if body.len() as u64 > MAX_DOCUMENT_BODY {
            bail!("cpscan: documento {ssc}: respuesta mayor de {MAX_DOCUMENT_BODY} bytes");
        }
        parse_document(ssc, &body)
    }
}

fn parse_document(ssc: &str, body: &[u8]) -> Result<Option<Document>> {
    let trimmed = body.trim_ascii();
    if trimmed == b"null" {
        return Ok(None);
    }
    if trimmed.first() != Some(&b'{') {
        bail!("cpscan: documento {ssc}: la respuesta no es un objeto JSON");
    }
    let value: Value = serde_json::from_slice(trimmed)
        .map_err(|e| anyhow!("cpscan: documento {ssc}: JSON invalido: {e}"))?;
    // The SSC is the key that was asked for; the echoed value is informational.
    Ok(Some(Document {
        ssc: ssc.to_owned(),
        origin: scalar_text(value.get("origen")),
        additional: scalar_text(value.get("adicional")),
        status: scalar_text(value.get("estado")),
    }))
}

/// Downloads CPScan files from the bucket (or any `http(s)` URL).
pub struct CpscanFiles {
    bucket: String,
    transport: Arc<CpscanTransport>,
}

impl CpscanFiles {
    pub fn new(bucket_url: &str, transport: Arc<CpscanTransport>) -> Result<Self> {
        let url = Url::parse(bucket_url.trim())
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
            .ok_or_else(|| anyhow!("cpscan: cpscan.bucket_url invalida"))?;
        Ok(Self {
            bucket: format!("{}/", url.as_str().trim_end_matches('/')),
            transport,
        })
    }

    /// Resolves a bucket file name or an absolute URL: bucket files and old
    /// `file/view` URLs become `<bucket>/<name>/download`. Only `http(s)` URLs
    /// are accepted.
    pub fn url_for(&self, reference: &str) -> Result<String> {
        let reference = reference.trim();
        if reference.is_empty() {
            bail!("cpscan: referencia de archivo vacia");
        }
        if !reference.contains("://") {
            return Ok(with_download(format!(
                "{}{}",
                self.bucket,
                reference.trim_start_matches('/')
            )));
        }
        let parsed = Url::parse(reference)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
        if parsed.is_none() {
            bail!("cpscan: URL no soportada {reference:?}");
        }
        let lower = reference.to_lowercase();
        if lower.contains(LEGACY_FILE_VIEW) {
            let without_query = reference.split('?').next().unwrap_or_default();
            let name = without_query
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or_default();
            return Ok(with_download(format!("{}{name}", self.bucket)));
        }
        if lower.starts_with(&self.bucket.to_lowercase()) {
            return Ok(with_download(reference.to_owned()));
        }
        Ok(reference.to_owned())
    }

    /// Downloads `reference`.
    pub fn download(&self, reference: &str) -> Result<Vec<u8>, FileError> {
        let url = self
            .url_for(reference)
            .map_err(|e| FileError::NoFile(format!("{e:#}")))?;
        let (status, body) = self
            .transport
            .get(&url, None, MAX_FILE_SIZE)
            .map_err(|e| FileError::Failed(format!("cpscan: {reference}: {e:#}")))?;
        if status != 200 {
            return Err(if status == 404 {
                FileError::NoFile(format!("cpscan: {reference}: HTTP 404"))
            } else {
                FileError::Failed(format!("cpscan: {reference}: HTTP {status}"))
            });
        }
        if body.len() as u64 > MAX_FILE_SIZE {
            return Err(FileError::Failed(format!(
                "cpscan: {reference}: archivo mayor de {MAX_FILE_SIZE} bytes"
            )));
        }
        if body.len() < MIN_FILE_SIZE {
            return Err(FileError::NoFile(format!(
                "cpscan: {reference}: ARCHIVO_VACIO"
            )));
        }
        Ok(body)
    }
}

fn with_download(url: String) -> String {
    if url.ends_with("/download") {
        url
    } else {
        format!("{url}/download")
    }
}

fn is_url(s: &str) -> bool {
    let lower = s.trim().to_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// The references to try, in order.
pub fn candidates(ssc: &str, doc: Option<&Document>) -> Vec<String> {
    let formula = format!("formula-{ssc}.jpg");
    let Some(doc) = doc else {
        return vec![formula];
    };
    let additional = doc.additional.trim();
    let origin = doc.origin_key();
    if is_url(additional) || (origin == "PC" && !additional.is_empty()) {
        return vec![additional.to_owned()];
    }
    match origin.as_str() {
        // PC_SIN_URL: the scanned PDF has no address.
        "PC" => Vec::new(),
        "SL" => vec![
            format!("firma-{ssc}.jpg"),
            format!("firma-{ssc}.png"),
            formula,
        ],
        // APP, BUCKET (treated exactly like APP), NULL and unknown origins.
        _ => vec![formula],
    }
}

/// Classifies a downloaded file by its bytes.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"%PDF") {
        return (bytes.len() >= MIN_PDF_SIZE).then_some("pdf");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Some("jpg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("gif");
    }
    if bytes.len() >= 14 && bytes.starts_with(b"BM") {
        return Some("bmp");
    }
    if bytes.starts_with(b"II*\x00") || bytes.starts_with(b"MM\x00*") {
        return Some("tif");
    }
    None
}

/// One completed lookup: the sidecar and, when it names a file, its bytes.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub sidecar: CpscanSidecar,
    pub payload: Vec<u8>,
}

/// Looks up the document of an SSC and downloads its support.
pub struct Acquirer {
    pub api: CpscanApi,
    pub files: CpscanFiles,
    /// Stamps `consultado` (the clock; replaceable in tests).
    pub now: fn() -> DateTime<Utc>,
}

impl Acquirer {
    pub fn new(api: CpscanApi, files: CpscanFiles) -> Self {
        Self {
            api,
            files,
            now: Utc::now,
        }
    }

    /// Resolves `ssc`. Definitive answers (no document, no usable file) come
    /// back as a [`Fetched`] without file; a lookup or download that could not
    /// complete is an `Err` and records nothing.
    pub fn fetch(&self, ssc: &str) -> Result<Fetched> {
        let doc = self.api.document(ssc)?;
        let mut sidecar = CpscanSidecar {
            version: SIDECAR_VERSION,
            ssc: ssc.to_owned(),
            fetched_at: (self.now)().to_rfc3339_opts(SecondsFormat::Secs, true),
            ..CpscanSidecar::default()
        };
        if let Some(doc) = &doc {
            sidecar.found = true;
            sidecar.origin = doc.origin.clone();
            sidecar.additional = doc.additional.trim().to_owned();
            sidecar.status = doc.status.clone();
        }
        for reference in candidates(ssc, doc.as_ref()) {
            let data = match self.files.download(&reference) {
                Ok(data) => data,
                Err(FileError::NoFile(_)) => continue,
                Err(FileError::Failed(message)) => bail!("{message}"),
            };
            if let Some(ext) = sniff(&data) {
                sidecar.file = reference;
                sidecar.ext = ext.to_owned();
                return Ok(Fetched {
                    sidecar,
                    payload: data,
                });
            }
        }
        Ok(Fetched {
            sidecar,
            payload: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MockResponse, MockServer};

    fn transport(limiter: Option<Arc<AdaptiveLimiter>>) -> Arc<CpscanTransport> {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("client");
        Arc::new(CpscanTransport::new(
            CpscanHttp::plain(client),
            limiter,
            RetryPolicy::instant(3),
        ))
    }

    fn acquirer(api: &MockServer, bucket: &MockServer) -> Acquirer {
        let t = transport(None);
        Acquirer::new(
            CpscanApi::new(&format!("{}/", api.url()), t.clone()).expect("api"),
            CpscanFiles::new(&format!("{}/files/", bucket.url()), t).expect("files"),
        )
    }

    fn jpg() -> Vec<u8> {
        let mut b = b"\xff\xd8\xff\xe0".to_vec();
        b.extend_from_slice(&[7; 40]);
        b
    }

    #[test]
    fn candidates_follow_the_decision_table() {
        let doc = |origin: &str, additional: &str| Document {
            ssc: "9".into(),
            origin: origin.into(),
            additional: additional.into(),
            status: String::new(),
        };
        assert_eq!(candidates("9", None), ["formula-9.jpg"]);
        for origin in ["APP", "bucket", "", "XYZ"] {
            assert_eq!(candidates("9", Some(&doc(origin, ""))), ["formula-9.jpg"]);
        }
        assert_eq!(
            candidates("9", Some(&doc("sl", ""))),
            ["firma-9.jpg", "firma-9.png", "formula-9.jpg"]
        );
        assert!(candidates("9", Some(&doc("PC", ""))).is_empty());
        assert_eq!(
            candidates("9", Some(&doc("PC", " scan.pdf "))),
            ["scan.pdf"]
        );
        assert_eq!(
            candidates("9", Some(&doc("SL", "https://h/x.jpg"))),
            ["https://h/x.jpg"]
        );
        assert_eq!(
            candidates("9", Some(&doc("APP", "https://h/x.jpg"))),
            ["https://h/x.jpg"]
        );
        // A non-URL adicional is ignored outside PC.
        assert_eq!(
            candidates("9", Some(&doc("APP", "abc.jpg"))),
            ["formula-9.jpg"]
        );
    }

    #[test]
    fn sniff_detects_the_kind_from_the_bytes() {
        let pdf = [b"%PDF-1.4".as_slice(), &[b' '; 100]].concat();
        assert_eq!(sniff(&pdf), Some("pdf"));
        assert_eq!(sniff(b"%PDF-1.4 tiny"), None);
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nxxxx"), Some("png"));
        assert_eq!(sniff(&jpg()), Some("jpg"));
        assert_eq!(sniff(b"GIF89a....."), Some("gif"));
        assert_eq!(sniff(b"BM12345678901234"), Some("bmp"));
        assert_eq!(sniff(b"II*\x00xxxx"), Some("tif"));
        assert_eq!(sniff(b"<html>error page</html>"), None);
    }

    #[test]
    fn the_sidecar_round_trips_and_rejects_bad_versions() {
        let sidecar = CpscanSidecar {
            version: 1,
            ssc: "123".into(),
            found: true,
            origin: "APP".into(),
            additional: String::new(),
            status: "OK".into(),
            file: "formula-123.jpg".into(),
            ext: "jpg".into(),
            fetched_at: "2026-10-23T14:05:09Z".into(),
        };
        let json = String::from_utf8(sidecar.to_json().expect("json")).expect("utf8");
        assert!(json.contains("\"documento\": true"), "{json}");
        assert!(
            json.contains("\"consultado\": \"2026-10-23T14:05:09Z\""),
            "{json}"
        );
        assert_eq!(
            CpscanSidecar::parse(json.as_bytes()).expect("parse"),
            sidecar
        );
        assert!(CpscanSidecar::parse(br#"{"version":2}"#).is_err());
        assert!(CpscanSidecar::parse(br#"{"version":1,"archivo":"a","ext":"exe"}"#).is_err());
        assert!(CpscanSidecar::parse(b"not json").is_err());
        assert!(CpscanSidecar::parse(br#"{"version":1,"archivo":""}"#).is_ok());
        let empty = CpscanSidecar {
            found: false,
            ssc: "5".into(),
            ..CpscanSidecar::default()
        };
        let json = String::from_utf8(empty.to_json().expect("json")).expect("utf8");
        assert!(!json.contains("consultado"), "omitted when empty: {json}");
        assert!(json.contains("\"archivo\": \"\""), "{json}");
    }

    #[test]
    fn warnings_name_the_ssc_and_the_origin() {
        let mut s = CpscanSidecar {
            ssc: "7".into(),
            ..CpscanSidecar::default()
        };
        assert_eq!(
            s.no_support_warning().as_deref(),
            Some("CPSCAN sin soporte (SSC 7, sin documento)")
        );
        s.found = true;
        s.origin = " pc ".into();
        assert_eq!(
            s.no_support_warning().as_deref(),
            Some("CPSCAN sin soporte (SSC 7, origen PC)")
        );
        s.file = "x".into();
        assert_eq!(s.no_support_warning(), None);
    }

    #[test]
    fn the_ssc_is_one_escaped_path_segment() {
        let t = transport(None);
        let api = CpscanApi::new("https://h.example/api/?q=1#f", t).expect("api");
        assert_eq!(
            api.document_url("123"),
            "https://h.example/api/documentos/123"
        );
        assert_eq!(
            api.document_url("a/b?x#y z"),
            "https://h.example/api/documentos/a%2Fb%3Fx%23y%20z"
        );
        assert_eq!(path_escape("a;b,c$&+=:@-_.~"), "a%3Bb%2Cc$&+=:@-_.~");
    }

    #[test]
    fn bucket_urls_are_normalized() {
        let files = CpscanFiles::new(
            "https://b.example/api/v1/buckets/cp/files/",
            transport(None),
        )
        .expect("files");
        let bucket = "https://b.example/api/v1/buckets/cp/files/";
        assert_eq!(
            files.url_for("formula-1.jpg").expect("url"),
            format!("{bucket}formula-1.jpg/download")
        );
        assert_eq!(
            files.url_for("/formula-1.jpg").expect("url"),
            format!("{bucket}formula-1.jpg/download")
        );
        assert_eq!(
            files
                .url_for("https://api-mobile.pharmaser.com.co/file/view/abc.pdf?x=1")
                .expect("url"),
            format!("{bucket}abc.pdf/download")
        );
        assert_eq!(
            files.url_for(&format!("{bucket}abc.pdf")).expect("url"),
            format!("{bucket}abc.pdf/download")
        );
        assert_eq!(
            files
                .url_for(&format!("{bucket}abc.pdf/download"))
                .expect("url"),
            format!("{bucket}abc.pdf/download")
        );
        assert_eq!(
            files.url_for("https://other.example/x.jpg").expect("url"),
            "https://other.example/x.jpg"
        );
        assert!(files.url_for("ftp://h/x.jpg").is_err());
        assert!(files.url_for("file:///etc/passwd").is_err());
        assert!(files.url_for(" ").is_err());
    }

    #[test]
    fn server_name_rewrite_keeps_the_port_and_the_host_header() {
        let url = Url::parse("https://10.0.0.5:8443/documentos/1").expect("url");
        let (rewritten, authority) =
            rewrite_for_server_name(&url, "cpscan.pharmaser.local").expect("rewrite");
        assert_eq!(
            rewritten,
            "https://cpscan.pharmaser.local:8443/documentos/1"
        );
        assert_eq!(authority, "10.0.0.5:8443");
    }

    #[test]
    fn api_answers_map_to_documents() {
        let cases: Vec<(MockResponse, bool)> = vec![
            (
                MockResponse::json(
                    200,
                    r#"{"ssc":"x","origen":"APP","adicional":null,"estado":7}"#,
                ),
                true,
            ),
            (MockResponse::json(404, ""), false),
            (MockResponse::json(204, ""), false),
            (MockResponse::json(200, "null"), false),
        ];
        for (response, found) in cases {
            let api = MockServer::sequence(vec![response]);
            let t = transport(None);
            let doc = CpscanApi::new(api.url(), t)
                .expect("api")
                .document("55")
                .expect("lookup");
            assert_eq!(doc.is_some(), found);
            assert_eq!(api.requests()[0].path, "/documentos/55");
            assert_eq!(api.requests()[0].header("accept"), Some("application/json"));
        }
        let api = MockServer::sequence(vec![MockResponse::json(
            200,
            r#"{"origen":"SL","adicional":" u ","estado":7}"#,
        )]);
        let doc = CpscanApi::new(api.url(), transport(None))
            .expect("api")
            .document("5")
            .expect("lookup")
            .expect("doc");
        assert_eq!(
            doc,
            Document {
                ssc: "5".into(),
                origin: "SL".into(),
                additional: "u".into(),
                status: "7".into()
            }
        );
    }

    #[test]
    fn api_failures_are_errors() {
        for response in [
            MockResponse::json(200, "[1]"),
            MockResponse::json(200, "{broken"),
            MockResponse::json(200, ""),
            MockResponse::json(403, "{}"),
            MockResponse::json(503, "{}"),
        ] {
            let api = MockServer::sequence(vec![response]);
            let result = CpscanApi::new(api.url(), transport(None))
                .expect("api")
                .document("5");
            assert!(result.is_err());
        }
        let api = MockServer::sequence(vec![MockResponse::json(503, "{}")]);
        let _ = CpscanApi::new(api.url(), transport(None))
            .expect("api")
            .document("5");
        assert_eq!(api.requests().len(), 3, "5xx is retried up to 3 attempts");
    }

    #[test]
    fn app_origin_downloads_the_formula_jpg() {
        let api = MockServer::sequence(vec![MockResponse::json(200, r#"{"origen":"APP"}"#)]);
        let bucket = MockServer::start(|req| {
            if req.path == "/files/formula-77.jpg/download" {
                MockResponse::bytes(200, jpg())
            } else {
                MockResponse::text(404, "")
            }
        });
        let fetched = acquirer(&api, &bucket).fetch("77").expect("fetch");
        assert_eq!(fetched.sidecar.file, "formula-77.jpg");
        assert_eq!(fetched.sidecar.ext, "jpg");
        assert!(fetched.sidecar.found);
        assert_eq!(fetched.payload, jpg());
        assert_eq!(fetched.sidecar.version, 1);
        assert!(fetched.sidecar.fetched_at.ends_with('Z'));
    }

    #[test]
    fn sl_tries_firma_jpg_then_png_then_the_formula() {
        let api = MockServer::sequence(vec![MockResponse::json(200, r#"{"origen":"SL"}"#)]);
        let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[0; 30]].concat();
        let png_body = png.clone();
        let bucket = MockServer::start(move |req| {
            if req.path == "/files/firma-8.png/download" {
                MockResponse::bytes(200, png_body.clone())
            } else if req.path == "/files/firma-8.jpg/download" {
                MockResponse::bytes(200, b"<html>not an image</html>".to_vec())
            } else {
                MockResponse::text(404, "")
            }
        });
        let fetched = acquirer(&api, &bucket).fetch("8").expect("fetch");
        assert_eq!(fetched.sidecar.file, "firma-8.png");
        assert_eq!(fetched.sidecar.ext, "png");
        let paths: Vec<String> = bucket.requests().into_iter().map(|r| r.path).collect();
        assert_eq!(
            paths,
            ["/files/firma-8.jpg/download", "/files/firma-8.png/download"]
        );
    }

    #[test]
    fn pc_without_url_and_missing_files_are_definitive_nothing() {
        let api = MockServer::sequence(vec![MockResponse::json(200, r#"{"origen":"PC"}"#)]);
        let bucket = MockServer::start(|_| MockResponse::text(404, ""));
        let fetched = acquirer(&api, &bucket).fetch("9").expect("fetch");
        assert!(!fetched.sidecar.has_file());
        assert!(fetched.sidecar.found);
        assert!(
            bucket.requests().is_empty(),
            "PC without adicional makes no download"
        );

        let api = MockServer::sequence(vec![MockResponse::text(404, "")]);
        let fetched = acquirer(&api, &bucket).fetch("9").expect("fetch");
        assert!(!fetched.sidecar.found);
        assert_eq!(bucket.requests().len(), 1, "no document tries the formula");
    }

    #[test]
    fn adicional_url_is_the_only_file_tried_and_non_http_is_rejected() {
        let bucket = MockServer::start(|req| {
            if req.path == "/x/scan.pdf" {
                let mut pdf = b"%PDF-1.4\n".to_vec();
                pdf.extend_from_slice(&[b'a'; 200]);
                MockResponse::bytes(200, pdf)
            } else {
                MockResponse::text(404, "")
            }
        });
        let api = MockServer::sequence(vec![MockResponse::json(
            200,
            &format!(
                r#"{{"origen":"PC","adicional":"{}/x/scan.pdf"}}"#,
                bucket.url()
            ),
        )]);
        let fetched = acquirer(&api, &bucket).fetch("3").expect("fetch");
        assert_eq!(fetched.sidecar.ext, "pdf");
        assert_eq!(
            fetched.sidecar.additional,
            format!("{}/x/scan.pdf", bucket.url())
        );
        assert_eq!(bucket.requests().len(), 1);

        let api = MockServer::sequence(vec![MockResponse::json(
            200,
            r#"{"origen":"PC","adicional":"ftp://h/x.pdf"}"#,
        )]);
        let fetched = acquirer(&api, &bucket).fetch("3").expect("fetch");
        assert!(!fetched.sidecar.has_file());
        assert_eq!(
            bucket.requests().len(),
            1,
            "ftp is rejected without a request"
        );
    }

    #[test]
    fn transient_download_failures_are_errors_but_404_and_tiny_bodies_are_not() {
        let api = MockServer::sequence(vec![MockResponse::json(200, r#"{"origen":"APP"}"#)]);
        let bucket = MockServer::sequence(vec![MockResponse::text(503, "")]);
        assert!(acquirer(&api, &bucket).fetch("1").is_err());
        let bucket = MockServer::sequence(vec![MockResponse::bytes(200, b"tiny".to_vec())]);
        let fetched = acquirer(&api, &bucket).fetch("1").expect("fetch");
        assert!(!fetched.sidecar.has_file());
        let bucket = MockServer::sequence(vec![MockResponse::bytes(
            200,
            b"<html>error page here</html>".to_vec(),
        )]);
        let fetched = acquirer(&api, &bucket).fetch("1").expect("fetch");
        assert!(
            !fetched.sidecar.has_file(),
            "a non-image body is not a support"
        );
    }

    #[test]
    fn lookups_and_downloads_share_the_limiter() {
        let limiter = Arc::new(AdaptiveLimiter::new(
            crate::http::limiter::specs::CPSCAN.options(),
        ));
        let api = MockServer::sequence(vec![MockResponse::json(200, r#"{"origen":"APP"}"#)]);
        let bucket = MockServer::sequence(vec![MockResponse::bytes(200, jpg())]);
        let t = transport(Some(limiter.clone()));
        let acquirer = Acquirer::new(
            CpscanApi::new(api.url(), t.clone()).expect("api"),
            CpscanFiles::new(&format!("{}/files/", bucket.url()), t).expect("files"),
        );
        acquirer.fetch("1").expect("fetch");
        assert_eq!(limiter.active(), 0, "every slot is given back");
    }
}
