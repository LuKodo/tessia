//! Medicar convenio report client (port of Go `internal/adapters/medicar`).
//!
//! One download is the following flow, every attempt with its own cookie jar:
//!
//! 1. `GET {base}/` and scrape the `csrf_token`;
//! 2. `POST {base}/procesar.php` with the identifier and the MD5-hex login and
//!    password (retried per request, see [`request_policy`]);
//! 3. `GET {products}/convenioArticulos.php` (opens the report page in the session);
//! 4. `POST {products}/exportarExcelConvenioArticulos.php` (multipart, 61 columns);
//! 5. `GET` the CSV link found in the export response and save it atomically.
//!
//! Transient failures (network errors, 408/425/429/5xx) restart the whole flow
//! with a fresh session and a doubling backoff; a rejected login, a missing
//! csrf token, a missing CSV link or any other 4xx is permanent and stops at
//! once. Error texts are Go's, verbatim.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::NaiveDate;
use md5::{Digest, Md5};
use regex::Regex;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{ACCEPT, CONTENT_TYPE, REFERER};
use url::Url;

use crate::config::Settings;
use crate::http::{self, ClientOptions, HttpError, RetryPolicy, send_with_retry};
use crate::util::fsx;

/// Java: 1200 s read timeout.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// Java: 7200 s export read timeout.
pub const DEFAULT_EXPORT_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
/// Whole-flow attempts.
pub const DEFAULT_ATTEMPTS: u32 = 3;
/// First whole-flow backoff; doubles per attempt.
pub const DEFAULT_RETRY_DELAY: Duration = Duration::from_secs(30);
/// Connection timeout (Go: 2 min dial timeout).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2 * 60);
/// Largest HTML page read.
const PAGE_LIMIT: u64 = 64 << 20;
/// Largest login answer read.
const LOGIN_LIMIT: u64 = 1 << 20;

const HTML_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
const EXPORT_ACCEPT: &str =
    "text/csv, application/csv, application/vnd.ms-excel, application/octet-stream, */*";
const CSV_ACCEPT: &str = "text/csv, application/csv, application/octet-stream, */*";
const LOGIN_ACCEPT: &str = "application/json, text/javascript, */*; q=0.01";

/// Per-request policy of the login: 4 attempts with about 1 s, 2 s and 4 s
/// backoffs, also for the POST (a repeated login only opens another session).
pub fn request_policy() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 4,
        base_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(10),
        retry_non_idempotent: true,
        ..RetryPolicy::default()
    }
}

/// Settings of the client.
#[derive(Clone)]
pub struct MedicarOptions {
    /// e.g. `https://host/pharmaser/mutualser`
    pub base_url: String,
    /// e.g. `https://host/pharmaser/productosDespachados`
    pub products_url: String,
    /// `f_id`
    pub identifier: String,
    pub username: String,
    pub password: String,
    /// Optional PEM CA bundle to pin the portal.
    pub ca_file: Option<String>,
    pub read_timeout: Duration,
    pub export_timeout: Duration,
    /// Whole-flow attempts.
    pub attempts: u32,
    /// First whole-flow backoff; doubles per attempt.
    pub retry_delay: Duration,
    /// Policy of the per-request retries of the login.
    pub request_policy: RetryPolicy,
}

impl std::fmt::Debug for MedicarOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MedicarOptions")
            .field("base_url", &self.base_url)
            .field("products_url", &self.products_url)
            .finish_non_exhaustive()
    }
}

impl MedicarOptions {
    /// Go `New`: URLs and credentials are required; trailing `/` is dropped.
    pub fn new(
        base_url: &str,
        products_url: &str,
        identifier: &str,
        username: &str,
        password: &str,
    ) -> Result<Self> {
        if base_url.trim().is_empty() || products_url.trim().is_empty() {
            bail!("medicar: base and products URLs are required");
        }
        if identifier.is_empty() || username.is_empty() || password.is_empty() {
            bail!("medicar: identifier, username and password are required");
        }
        Ok(Self {
            base_url: base_url.trim().trim_end_matches('/').to_owned(),
            products_url: products_url.trim().trim_end_matches('/').to_owned(),
            identifier: identifier.to_owned(),
            username: username.to_owned(),
            password: password.to_owned(),
            ca_file: None,
            read_timeout: DEFAULT_READ_TIMEOUT,
            export_timeout: DEFAULT_EXPORT_TIMEOUT,
            attempts: DEFAULT_ATTEMPTS,
            retry_delay: DEFAULT_RETRY_DELAY,
            request_policy: request_policy(),
        })
    }

    /// Reads the `medicar.*` keys. The error names the missing keys and their
    /// environment variables, never values.
    pub fn from_settings(settings: &Settings) -> Result<Self> {
        settings.require(&[
            "medicar.base_url",
            "medicar.products_url",
            "medicar.identifier",
            "medicar.username",
            "medicar.password",
        ])?;
        let get = |key: &str| settings.get(key).unwrap_or_default();
        let mut options = Self::new(
            get("medicar.base_url"),
            get("medicar.products_url"),
            get("medicar.identifier"),
            get("medicar.username"),
            get("medicar.password"),
        )?;
        options.ca_file = settings.get("medicar.ca_file").map(str::to_owned);
        Ok(options)
    }

    /// No waiting between attempts (tests): tiny whole-flow and login backoffs.
    #[must_use]
    pub fn with_fast_retries(mut self) -> Self {
        self.retry_delay = Duration::from_millis(1);
        self.request_policy = RetryPolicy {
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
            jitter: 0.0,
            ..request_policy()
        };
        self
    }
}

/// Saved report name: `Convenio_<ddMMyyyy>_<ddMMyyyy>.csv`.
pub fn file_name(from: NaiveDate, to: NaiveDate) -> String {
    format!(
        "Convenio_{}_{}.csv",
        from.format("%d%m%Y"),
        to.format("%d%m%Y")
    )
}

/// One multipart form field.
pub type Field = (String, String);

/// Requested columns, in order (`<key>|<label>`).
const EXPORT_COLUMNS: [&str; 61] = [
    "p_nit_convenio2|NIT",
    "p_plan|Plan",
    "p_splan|SubPlan",
    "p_factura|Factura",
    "p_factura2|Factura MC",
    "p_cuenta|Factura consolidada",
    "tipoDoc|Tipo de identificacion",
    "id_paci|No. Documento",
    "p_beneficiario|Beneficiario",
    "cat|Categoria actual",
    "p_sucursal|Pto. venta",
    "p_policia|No. Formula",
    "id_formula|No. SSC",
    "p_tipo_formula|Tipo formula",
    "p_fecha|Fecha",
    "lote|Lote",
    "p_plu|PLU",
    "articulo|Articulo",
    "p_cantidad|Cantidad",
    "valor_unitario|Vlr. unitario",
    "costo_unitario|Costo unitario",
    "valor_total|Vlr. total",
    "cuota|Cuota",
    "margen|Margen",
    "estado|Estado",
    "usdespa|Usuario despacha",
    "nombredespa|Nombre usuario despacha",
    "autorizacion|Numero autorizacion",
    "autorizacion_ctc|Mipres",
    "observacion|Observaciones",
    "fechaDescarga|Fecha de descarga",
    "fechaElaboro|Fecha elaboracion factura",
    "login_cuenta|Usuario elabora",
    "usuario_cuenta|Nombre usuario elabora",
    "codigo_cliente|CUM",
    "num_entregas|Numero entregas",
    "p_cant_diaria|Cantidad diaria",
    "fechaSiguiente|Fecha proxima entrega",
    "autorizacion_medicamento|Autorizacion Medicamento",
    "factura_homologo|Factura homologo",
    "codigo_homologo|Cod. homologo",
    "descripcion_homologo|Descripcion homologo",
    "cantidad_homologo|Cantidad homologo",
    "valor_unitario_homologo|Vlr. unitario homologo",
    "modalidad|Modalidad",
    "fecha_formula|Fecha formula",
    "fecha_autorizacion|Fecha autorizacion",
    "diag|Codigo diagnostico",
    "nombre_diag|Nombre diagnostico",
    "domicilio|Domicilio",
    "direccion|Direccion entrega",
    "telefono|Telefono entrega",
    "causal_exenta|Causal exento",
    "ips|IPS",
    "id_medico|Id Medico",
    "nombre_medico|Nombre Medico",
    "por_iva|Porcentaje Iva",
    "sexo|Genero Paciente",
    "fecha_de_nacimiento|Fecha Nacimiento",
    "depto|Departamento",
    "ciudad|Municipio",
];

/// The export form, in the exact order the portal expects.
pub fn export_fields(from: NaiveDate, to: NaiveDate) -> Vec<Field> {
    let field = |name: &str, value: &str| (name.to_owned(), value.to_owned());
    let mut fields = vec![
        field("cual_convenio", "este"),
        field("convenios[]", "0"),
        field("f_clinit", "1"),
        field("filtrar", "1"),
        field("f_fecha", &from.format("%Y-%m-%d").to_string()),
        field("f_fecha_final", &to.format("%Y-%m-%d").to_string()),
    ];
    for plan in ["6", "4", "3", "2", "8", "9"] {
        fields.push(field("f_splan[]", plan));
    }
    fields.push(field("f_sucursal[]", "todos"));
    fields.push(field("f_tipo_formula[]", "todos"));
    fields.push(field("control_columnas_a_exportar", "todos"));
    for (index, column) in EXPORT_COLUMNS.iter().enumerate() {
        fields.push(field(
            &format!("columnas_a_exportar[{}][]", index + 1),
            column,
        ));
    }
    fields.push(field("descarga", "ahora"));
    fields
}

/// Go `multipart.Writer.WriteField` for every field: `Content-Disposition`
/// only, no content type, CRLF line ends. Returns the body and its content type.
pub fn multipart_body(fields: &[Field]) -> (Vec<u8>, String) {
    let boundary = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let mut body = String::new();
    for (name, value) in fields {
        let _ = write!(
            body,
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        );
    }
    let _ = write!(body, "--{boundary}--\r\n");
    (
        body.into_bytes(),
        format!("multipart/form-data; boundary={boundary}"),
    )
}

static CSRF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)name=["']csrf_token["'][^>]*value=["']([^"']+)["']"#)
        .unwrap_or_else(|_| unreachable!("constant regex"))
});
static CSRF_ALT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)value=["']([^"']+)["'][^>]*name=["']csrf_token["']"#)
        .unwrap_or_else(|_| unreachable!("constant regex"))
});
static PARENT_LOCATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)parent\.location\s*=\s*["']([^"']+\.csv[^"']*)["']"#)
        .unwrap_or_else(|_| unreachable!("constant regex"))
});
static ABSOLUTE_CSV_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(https?://[^"'<>\s]+\.csv[^"'<>\s]*)"#)
        .unwrap_or_else(|_| unreachable!("constant regex"))
});

/// The (entity-decoded) csrf token of the login page, attributes in either order.
pub fn find_csrf(page: &str) -> Option<String> {
    let captures = CSRF_RE
        .captures(page)
        .or_else(|| CSRF_ALT_RE.captures(page))?;
    Some(unescape_html(&captures[1]))
}

/// Minimal HTML entity decoding (Go `html.UnescapeString` for the common
/// named entities and numeric references).
fn unescape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest
            .find(';')
            .filter(|end| *end <= 10)
            .and_then(|end| decode_entity(&rest[1..end]).map(|c| (c, end + 1)));
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Lowercase MD5 hex digest (the portal expects MD5 credentials).
pub fn md5_hex(text: &str) -> String {
    hex::encode(Md5::digest(text.as_bytes()))
}

/// The report link of the export response, resolved against the products base
/// when relative.
pub fn csv_url(body: &str, products_base: &str) -> Result<String> {
    let link = PARENT_LOCATION_RE
        .captures(body)
        .or_else(|| ABSOLUTE_CSV_RE.captures(body))
        .map(|c| c[1].trim().to_owned())
        .unwrap_or_default();
    if link.is_empty() {
        bail!(
            "medicar: the export response has no CSV link (no data for the range, or the portal changed)"
        );
    }
    if link.starts_with("http://") || link.starts_with("https://") {
        return Ok(link);
    }
    let base = Url::parse(&format!("{}/", products_base.trim_end_matches('/')))
        .context("medicar: invalid products URL")?;
    let resolved = base
        .join(&link)
        .map_err(|err| anyhow!("medicar: invalid CSV link: {err}"))?;
    Ok(resolved.to_string())
}

fn transient_status(code: u16) -> bool {
    matches!(code, 408 | 425 | 429) || code >= 500
}

/// How one attempt failed.
enum FlowError {
    /// A retry cannot fix it.
    Permanent(anyhow::Error),
    /// Worth a whole new session.
    Transient(anyhow::Error),
}

fn permanent(message: impl Into<String>) -> FlowError {
    FlowError::Permanent(anyhow!(message.into()))
}

fn transient(message: impl Into<String>) -> FlowError {
    FlowError::Transient(anyhow!(message.into()))
}

/// Removes its file on drop: a partial download never stays behind.
struct PartFile(PathBuf);

impl Drop for PartFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Downloads convenio reports.
#[derive(Debug, Clone)]
pub struct MedicarClient {
    opts: MedicarOptions,
}

impl MedicarClient {
    pub fn new(opts: MedicarOptions) -> Result<Self> {
        if opts.base_url.is_empty() || opts.products_url.is_empty() {
            bail!("medicar: base and products URLs are required");
        }
        Ok(Self { opts })
    }

    /// Exports the convenio for `[from, to]` and saves it at `dest` atomically
    /// (a `.convenio-*.part` file in the same folder, then a rename).
    pub fn download(&self, from: NaiveDate, to: NaiveDate, dest: &Path) -> Result<()> {
        if to < from {
            bail!("medicar: the end date is before the start date");
        }
        let attempts = self.opts.attempts.max(1);
        let mut delay = self.opts.retry_delay;
        let mut last = anyhow!("medicar: download did not run");
        for attempt in 1..=attempts {
            if crate::shutdown::requested() {
                bail!("medicar: descarga interrumpida");
            }
            match self.attempt(from, to, dest) {
                Ok(()) => return Ok(()),
                Err(FlowError::Permanent(err)) => return Err(err),
                Err(FlowError::Transient(err)) => last = err,
            }
            if attempt == attempts {
                break;
            }
            tracing::warn!(
                attempt,
                error = %last,
                wait_ms = delay.as_millis() as u64,
                "medicar download failed, retrying with a new session"
            );
            sleep_interruptible(delay);
            delay = delay.saturating_mul(2);
        }
        Err(last)
    }

    /// One whole flow with a fresh cookie jar.
    fn attempt(&self, from: NaiveDate, to: NaiveDate, dest: &Path) -> Result<(), FlowError> {
        let o = &self.opts;
        let mut options = ClientOptions::default()
            .with_cookies()
            .with_user_agent(http::MEDICAR_USER_AGENT)
            .with_ca_file(o.ca_file.as_deref())
            .with_timeout(o.export_timeout + Duration::from_secs(60));
        options.connect_timeout = CONNECT_TIMEOUT;
        let client = http::build_client(&options).map_err(FlowError::Permanent)?;

        let page = self.text(
            client
                .get(format!("{}/", o.base_url))
                .header(ACCEPT, HTML_ACCEPT),
            "GET /",
            o.read_timeout,
        )?;
        let csrf = find_csrf(&page)
            .ok_or_else(|| permanent("medicar: csrf_token not found in the login page"))?;
        self.login(&client, &csrf)?;

        let products_page = format!("{}/convenioArticulos.php", o.products_url);
        self.text(
            client
                .get(&products_page)
                .header(ACCEPT, HTML_ACCEPT)
                .header(
                    REFERER,
                    format!("{}/el_admin/contenedor_control.php", o.base_url),
                ),
            "GET /convenioArticulos.php",
            o.read_timeout,
        )?;

        let (body, content_type) = multipart_body(&export_fields(from, to));
        let progress = self.text(
            client
                .post(format!(
                    "{}/exportarExcelConvenioArticulos.php",
                    o.products_url
                ))
                .header(ACCEPT, EXPORT_ACCEPT)
                .header(REFERER, products_page)
                .header(CONTENT_TYPE, content_type)
                .body(body),
            "POST /exportarExcelConvenioArticulos.php",
            o.export_timeout,
        )?;
        let link = csv_url(&progress, &o.products_url).map_err(FlowError::Permanent)?;
        self.save(&client, &link, dest)
    }

    fn login(&self, client: &Client, csrf: &str) -> Result<(), FlowError> {
        let o = &self.opts;
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("f_id", &o.identifier)
            .append_pair("f_log", &md5_hex(&o.username))
            .append_pair("f_pass", &md5_hex(&o.password))
            .append_pair("f_pass_nuevo", "")
            .append_pair("f_pass_confirma", "")
            .append_pair("csrf_token", csrf)
            .finish();
        let target = format!("{}/procesar.php", o.base_url);
        let referer = format!("{}/", o.base_url);
        let response = send_with_retry(&o.request_policy, None, || {
            client
                .post(&target)
                .header(ACCEPT, LOGIN_ACCEPT)
                .header(
                    CONTENT_TYPE,
                    "application/x-www-form-urlencoded; charset=UTF-8",
                )
                .header(REFERER, &referer)
                .header("X-Requested-With", "XMLHttpRequest")
                .body(form.clone())
                .timeout(o.read_timeout)
        })
        .map_err(|err| match err {
            HttpError::Transport { source, .. } => transient(format!("medicar: login: {source}")),
            HttpError::Build(source) => permanent(format!("medicar: login: {source}")),
        })?;
        let status = response.status().as_u16();
        let raw = read_limited(response, LOGIN_LIMIT)
            .map_err(|err| FlowError::Permanent(err.context("medicar: login: read body")))?;
        if transient_status(status) {
            return Err(transient(format!("medicar: login: HTTP {status}")));
        }
        if status != 200 || !String::from_utf8_lossy(&raw).contains("\"status\"") {
            return Err(permanent(format!(
                "medicar: login was not accepted (HTTP {status})"
            )));
        }
        Ok(())
    }

    /// One request returning the body; 4xx is permanent, 5xx and transport
    /// errors transient.
    fn text(
        &self,
        request: RequestBuilder,
        what: &str,
        timeout: Duration,
    ) -> Result<String, FlowError> {
        let response = request
            .timeout(timeout)
            .send()
            .map_err(|err| transient(format!("medicar: {what}: {}", err.without_url())))?;
        let status = response.status().as_u16();
        let raw = read_limited(response, PAGE_LIMIT).map_err(|err| {
            FlowError::Transient(err.context(format!("medicar: {what}: read body")))
        })?;
        if status != 200 {
            let message = format!("medicar: {what}: HTTP {status}");
            return Err(if transient_status(status) {
                transient(message)
            } else {
                permanent(message)
            });
        }
        Ok(String::from_utf8_lossy(&raw).into_owned())
    }

    /// Downloads `link` to `dest` atomically.
    fn save(&self, client: &Client, link: &str, dest: &Path) -> Result<(), FlowError> {
        let mut response = client
            .get(link)
            .header(ACCEPT, CSV_ACCEPT)
            .timeout(self.opts.read_timeout)
            .send()
            .map_err(|err| transient(format!("medicar: download CSV: {}", err.without_url())))?;
        let status = response.status().as_u16();
        if status != 200 {
            let message = format!("medicar: download CSV: HTTP {status}");
            return Err(if transient_status(status) {
                transient(message)
            } else {
                permanent(message)
            });
        }
        let parent = dest.parent().unwrap_or_else(|| Path::new("."));
        fsx::ensure_dir(parent).map_err(FlowError::Permanent)?;
        let part = PartFile(parent.join(format!(
            ".convenio-{}.part",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        )));
        fsx::write_private(&part.0, b"").map_err(FlowError::Permanent)?;
        let mut file = OpenOptions::new()
            .write(true)
            .open(&part.0)
            .map_err(|err| permanent(format!("medicar: download CSV: {err}")))?;
        std::io::copy(&mut response, &mut file)
            .map_err(|err| transient(format!("medicar: download CSV: {err}")))?;
        file.flush()
            .and_then(|()| file.sync_all())
            .map_err(|err| FlowError::Transient(anyhow!(err)))?;
        drop(file);
        fsx::move_file(&part.0, dest).map_err(FlowError::Transient)
    }
}

fn read_limited(response: reqwest::blocking::Response, limit: u64) -> Result<Vec<u8>> {
    let mut raw = Vec::new();
    response.take(limit).read_to_end(&mut raw)?;
    Ok(raw)
}

/// Sleeps `total`, waking early once an interrupt was requested.
fn sleep_interruptible(total: Duration) {
    const STEP: Duration = Duration::from_millis(200);
    let mut left = total;
    while !left.is_zero() && !crate::shutdown::requested() {
        let step = left.min(STEP);
        std::thread::sleep(step);
        left -= step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("date")
    }

    #[test]
    fn file_name_uses_ddmmyyyy() {
        assert_eq!(
            file_name(day(2026, 10, 1), day(2026, 10, 21)),
            "Convenio_01102026_21102026.csv"
        );
    }

    #[test]
    fn csrf_is_found_with_either_attribute_order_and_decoded() {
        let name_first = r#"<input type="hidden" name="csrf_token" value="a&amp;b">"#;
        let value_first = r#"<input type='hidden' VALUE='a&amp;b' NAME='csrf_token'>"#;
        assert_eq!(find_csrf(name_first).as_deref(), Some("a&b"));
        assert_eq!(find_csrf(value_first).as_deref(), Some("a&b"));
        assert_eq!(find_csrf("<form></form>"), None);
        assert_eq!(unescape_html("x&#65;&#x42;&bogus;&"), "xAB&bogus;&");
    }

    #[test]
    fn credentials_are_md5_hex() {
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn csv_url_prefers_parent_location_and_resolves_relative_links() {
        let products = "https://h.example/pharmaser/productosDespachados/";
        let relative = "<script>parent.location='descargas/c.csv?x=1';</script>";
        assert_eq!(
            csv_url(relative, products).expect("relative"),
            "https://h.example/pharmaser/productosDespachados/descargas/c.csv?x=1"
        );
        let absolute_only = "ok https://other.example/a/b.CSV done";
        assert_eq!(
            csv_url(absolute_only, products).expect("absolute"),
            "https://other.example/a/b.CSV"
        );
        let both = "https://other.example/a.csv parent.location = \"/rooted/x.csv\"";
        assert_eq!(
            csv_url(both, products).expect("parent first"),
            "https://h.example/rooted/x.csv"
        );
        let err = csv_url("<html>no data</html>", products).expect_err("no link");
        assert!(err.to_string().contains("no CSV link"), "{err}");
    }

    #[test]
    fn export_fields_have_the_fixed_shape() {
        let fields = export_fields(day(2026, 10, 1), day(2026, 10, 21));
        assert_eq!(fields.len(), 6 + 6 + 3 + 61 + 1);
        assert_eq!(fields[4], ("f_fecha".into(), "2026-10-01".into()));
        assert_eq!(fields[5], ("f_fecha_final".into(), "2026-10-21".into()));
        assert_eq!(fields.last(), Some(&("descarga".into(), "ahora".into())));
    }

    #[test]
    fn multipart_body_matches_go_write_field() {
        let (body, content_type) = multipart_body(&[("a[]".into(), "1".into())]);
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .expect("boundary");
        let expected = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"a[]\"\r\n\r\n1\r\n--{boundary}--\r\n"
        );
        assert_eq!(String::from_utf8(body).expect("utf8"), expected);
    }

    #[test]
    fn options_require_urls_and_credentials() {
        assert!(MedicarOptions::new("", "p", "i", "u", "x").is_err());
        assert!(MedicarOptions::new("b", "p", "i", "", "x").is_err());
        let ok = MedicarOptions::new("https://b/", "https://p/", "i", "u", "x").expect("ok");
        assert_eq!(ok.base_url, "https://b");
        assert_eq!(ok.products_url, "https://p");
        assert!(!format!("{ok:?}").contains("password"));
    }
}
