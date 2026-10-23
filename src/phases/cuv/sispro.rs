//! The client of the Ministerio de Salud FEV-RIPS service (SISPRO, "MinisterioLocal"
//! in Java), as used by `EnvioRIPS_Financiera_Extra` and
//! `ModificarTXT_JSON_AplicacionLocal` (Go `internal/adapters/sispro`):
//!
//! ```text
//! POST {base}/api/Auth/LoginSISPRO                 -> {"token": JWT}
//! POST {base}/api/PaquetesFevRips/CargarFevRips     Bearer; body {"rips","xmlFevFile"}
//! POST {base}/api/ConsultasFevRips/ConsultarCUV     Bearer; {"codigoUnicoValidacion"} -> ProcesoId
//! ```
//!
//! * The token is cached until one minute before its JWT `exp`, like
//!   `obtenerTokenMinisterio`; concurrent callers share one login.
//! * Only the login is retried (3 tries, doubling backoff, +-25 % jitter) on
//!   transient failures. `CargarFevRips` is **never** retried: it is not
//!   idempotent and Java did not retry it either.
//! * One [`AdaptiveLimiter`] (8/20/40, window 80) paces every exchange: the login
//!   tries, `CargarFevRips` and `ConsultarCUV`. The slot is taken right before the
//!   request and freed once the body is read, so the 210 s request timeout only
//!   starts after the slot is granted and a slot is not held while sleeping
//!   between login tries.
//! * TLS is verified; see [`super::tls`] for `minsalud.ca_file` and
//!   `minsalud.tls_pin_sha256`.

use std::io::Read;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime};

use anyhow::{Result, bail};
use rand::Rng;
use regex::Regex;
use reqwest::blocking::Client;
use url::Url;

use super::tls::build_sispro_client;
use crate::config::Settings;
use crate::http::token::{FetchedToken, TokenError};
use crate::http::{AdaptiveLimiter, TokenCache, jwt_expiry};

const LOGIN_PATH: &str = "/api/Auth/LoginSISPRO";
const CARGAR_PATH: &str = "/api/PaquetesFevRips/CargarFevRips";
const CONSULTA_PATH: &str = "/api/ConsultasFevRips/ConsultarCUV";
const MAX_RESPONSE: u64 = 16 << 20;
const SNIPPET_MAX: usize = 200;

/// The `CargarFevRips` request timeout (Java: 210 s), counted after the slot.
pub const CARGAR_TIMEOUT: Duration = Duration::from_secs(210);
/// Default login tries.
pub const DEFAULT_LOGIN_ATTEMPTS: usize = 3;
/// Default wait before the second login try; it doubles on every further retry.
pub const DEFAULT_LOGIN_BACKOFF: Duration = Duration::from_millis(500);

/// Hook that replaces the real sleep between login tries (tests).
pub type SleepFn = Arc<dyn Fn(Duration) + Send + Sync>;

/// Everything the client needs; the credentials come only from `minsalud.*`.
#[derive(Clone)]
pub struct SisproOptions {
    /// `scheme://host[:port]` (`minsalud.base_url`).
    pub base_url: String,
    pub tipo_doc: String,
    pub numero_doc: String,
    pub clave: String,
    pub nit: String,
    pub ca_file: Option<String>,
    pub tls_pin_sha256: Option<String>,
    /// Request timeout (default [`CARGAR_TIMEOUT`]).
    pub timeout: Duration,
    /// Login tries (0 = default 3; 1 disables retries).
    pub login_attempts: usize,
    /// Base wait before the second login try.
    pub login_backoff: Duration,
    pub sleep: Option<SleepFn>,
}

impl SisproOptions {
    /// Reads `minsalud.*` and fails naming the missing keys (never values).
    pub fn from_settings(settings: &Settings) -> Result<Self> {
        settings.require(&[
            "minsalud.base_url",
            "minsalud.tipo_doc",
            "minsalud.numero_doc",
            "minsalud.clave",
            "minsalud.nit",
        ])?;
        let get = |key: &str| settings.get(key).unwrap_or_default().to_owned();
        Ok(Self {
            base_url: get("minsalud.base_url"),
            tipo_doc: get("minsalud.tipo_doc"),
            numero_doc: get("minsalud.numero_doc"),
            clave: get("minsalud.clave"),
            nit: get("minsalud.nit"),
            ca_file: settings.get("minsalud.ca_file").map(str::to_owned),
            tls_pin_sha256: settings.get("minsalud.tls_pin_sha256").map(str::to_owned),
            timeout: CARGAR_TIMEOUT,
            login_attempts: DEFAULT_LOGIN_ATTEMPTS,
            login_backoff: DEFAULT_LOGIN_BACKOFF,
            sleep: None,
        })
    }
}

/// Failure of a request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SisproError {
    /// The login did not return a token (Java: "Token vacío").
    #[error("sispro: token vacio: {0}")]
    Token(String),
    /// Network error, timeout or unreadable response.
    #[error("{0}")]
    Transport(String),
}

impl SisproError {
    pub fn is_token(&self) -> bool {
        matches!(self, SisproError::Token(_))
    }
}

struct Shared {
    http: Client,
    base: Url,
    options: SisproOptions,
    limiter: Option<Arc<AdaptiveLimiter>>,
}

/// Talks to SISPRO. Cheap to share between threads (`&SisproClient`).
pub struct SisproClient {
    shared: Arc<Shared>,
    tokens: TokenCache,
}

impl std::fmt::Debug for SisproClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SisproClient")
            .field("host", &self.shared.base.host_str())
            .finish_non_exhaustive()
    }
}

impl SisproClient {
    /// Validates the options and builds the HTTP client.
    pub fn new(options: SisproOptions, limiter: Option<Arc<AdaptiveLimiter>>) -> Result<Self> {
        let base = parse_base_url(&options.base_url)?;
        let http = build_sispro_client(
            options.ca_file.as_deref(),
            options.tls_pin_sha256.as_deref(),
            options.timeout,
        )?;
        let shared = Arc::new(Shared {
            http,
            base,
            options,
            limiter,
        });
        let for_login = shared.clone();
        let tokens = TokenCache::new(Duration::from_secs(60), move || for_login.login());
        Ok(Self { shared, tokens })
    }

    /// Posts one `CargarFevRips` payload and returns the HTTP status and body.
    /// Never retried.
    pub fn cargar_fev_rips(&self, payload: Vec<u8>) -> Result<(u16, Vec<u8>), SisproError> {
        let token = self.token()?;
        self.shared
            .post(CARGAR_PATH, Some(&token), "application/json", payload)
            .map_err(SisproError::Transport)
    }

    /// `consultarProcesoId`: the `ProcesoId` of a CUV, or `None` on any failure.
    pub fn proceso_id(&self, cuv: &str) -> Option<i64> {
        let token = self.token().ok()?;
        let body = serde_json::to_vec(&serde_json::json!({ "codigoUnicoValidacion": cuv })).ok()?;
        let (status, raw) = self
            .shared
            .post(
                CONSULTA_PATH,
                Some(&token),
                "application/json; charset=UTF-8",
                body,
            )
            .ok()?;
        if !(200..=299).contains(&status) {
            return None;
        }
        parse_proceso_id(&raw)
    }

    fn token(&self) -> Result<String, SisproError> {
        self.tokens
            .token()
            .map_err(|TokenError::Fetch(msg)| SisproError::Token(msg))
    }
}

fn parse_base_url(raw: &str) -> Result<Url> {
    let trimmed = raw.trim().trim_end_matches('/');
    let valid = Url::parse(trimmed).ok().filter(|u| {
        matches!(u.scheme(), "http" | "https")
            && u.host_str().is_some()
            && u.path().trim_matches('/').is_empty()
            && u.query().is_none()
    });
    match valid {
        Some(url) => Ok(url),
        None => bail!("sispro: la URL base debe ser esquema://host[:puerto] (minsalud.base_url)"),
    }
}

/// `ProcesoId` of a `ConsultarCUV` answer (a number or a numeric text).
fn parse_proceso_id(raw: &[u8]) -> Option<i64> {
    let serde_json::Value::Object(map) = serde_json::from_slice(raw).ok()? else {
        return None;
    };
    let text = match map.get("ProcesoId")? {
        serde_json::Value::String(s) => s.trim().to_owned(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    text.parse::<i64>()
        .ok()
        .or_else(|| text.parse::<f64>().ok().map(|f| f as i64))
}

impl Shared {
    fn host(&self) -> String {
        let host = self.base.host_str().unwrap_or_default();
        match self.base.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        }
    }

    /// One exchange under a limiter slot; the body is read before the slot is
    /// released.
    fn post(
        &self,
        path: &str,
        token: Option<&str>,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<(u16, Vec<u8>), String> {
        let url = format!("{}{path}", self.base.as_str().trim_end_matches('/'));
        let mut request = self
            .http
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if path == CONSULTA_PATH {
            request = request.header(reqwest::header::ACCEPT, "application/json");
        }
        let host = self.host();
        let exchange = move || -> Result<(u16, (u16, Vec<u8>)), String> {
            let response = request
                .send()
                .map_err(|e| format!("sispro: POST {host}{path}: {}", error_chain(&e)))?;
            let status = response.status().as_u16();
            let mut raw = Vec::new();
            response
                .take(MAX_RESPONSE)
                .read_to_end(&mut raw)
                .map_err(|e| format!("sispro: read response: {e}"))?;
            Ok((status, (status, raw)))
        };
        match &self.limiter {
            Some(limiter) => limiter.exchange(exchange),
            None => exchange().map(|(_, value)| value),
        }
    }

    /// Obtains a token, retrying transient failures (network errors, HTTP
    /// 5xx/429 and a 2xx without token) with exponential backoff and jitter.
    /// Credential-style rejections (other 4xx) and a malformed token fail at
    /// once.
    fn login(&self) -> Result<FetchedToken, String> {
        let attempts = match self.options.login_attempts {
            0 => DEFAULT_LOGIN_ATTEMPTS,
            n => n,
        };
        let mut tries = 1;
        loop {
            let failure = match self.login_once() {
                Ok(token) => return Ok(token),
                Err(failure) => failure,
            };
            tracing::warn!(
                attempt = tries,
                of = attempts,
                retry = failure.retry,
                host = %self.host(),
                error = %failure.message,
                "SISPRO login failed"
            );
            if !failure.retry {
                return Err(failure.message);
            }
            if tries >= attempts {
                return Err(if attempts > 1 {
                    format!(
                        "sispro: login fallo tras {attempts} intentos: {}",
                        failure.message
                    )
                } else {
                    failure.message
                });
            }
            if crate::shutdown::requested() {
                return Err(failure.message);
            }
            self.sleep(self.backoff(tries));
            tries += 1;
        }
    }

    fn login_once(&self) -> Result<FetchedToken, LoginFailure> {
        let o = &self.options;
        let body = serde_json::json!({
            "persona": { "identificacion": { "tipo": o.tipo_doc, "numero": o.numero_doc } },
            "clave": o.clave,
            "nit": o.nit,
        });
        let body = serde_json::to_vec(&body).map_err(|e| LoginFailure::fatal(e.to_string()))?;
        let (status, raw) = self
            .post(LOGIN_PATH, None, "application/json", body)
            .map_err(LoginFailure::transient)?;
        let token = serde_json::from_slice::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| v.get("token").and_then(|t| t.as_str().map(str::to_owned)))
            .filter(|t| !t.trim().is_empty());
        let Some(token) = token else {
            let transient = status >= 500 || status == 429 || (200..300).contains(&status);
            return Err(LoginFailure {
                retry: transient,
                message: format!(
                    "sispro: login HTTP {status} sin token ({})",
                    self.body_snippet(&raw)
                ),
            });
        };
        let expires_at: SystemTime =
            jwt_expiry(&token).map_err(|e| LoginFailure::fatal(format!("sispro: token: {e}")))?;
        Ok(FetchedToken {
            token,
            expires_at: Some(expires_at),
        })
    }

    /// The wait after the `n`-th failed try: `base * 2^(n-1)`, +-25 %.
    fn backoff(&self, n: usize) -> Duration {
        let base = if self.options.login_backoff.is_zero() {
            DEFAULT_LOGIN_BACKOFF
        } else {
            self.options.login_backoff
        };
        let doubled = base.saturating_mul(1u32 << (n - 1).min(16));
        doubled.mul_f64(0.75 + rand::rng().random::<f64>() / 2.0)
    }

    fn sleep(&self, wait: Duration) {
        match &self.options.sleep {
            Some(sleep) => sleep(wait),
            None => std::thread::sleep(wait),
        }
    }

    /// A short, single-line, secret-free excerpt of a login response for error
    /// messages: it masks token-like fields, JWT-shaped strings and the
    /// configured password in case the server echoes them.
    fn body_snippet(&self, raw: &[u8]) -> String {
        let mut text = String::from_utf8_lossy(raw).replace('\u{FFFD}', "");
        let clave = &self.options.clave;
        if clave.len() >= 4 {
            text = text.replace(clave.as_str(), "[redacted]");
        }
        let text = SECRET_FIELD.replace_all(&text, "${1}[redacted]");
        let text = JWT_LIKE.replace_all(&text, "[redacted]");
        let words: Vec<&str> = text
            .split(|c: char| c.is_control() || c.is_whitespace())
            .filter(|w| !w.is_empty())
            .collect();
        let mut one_line = words.join(" ");
        if one_line.is_empty() {
            return "cuerpo vacio".into();
        }
        if one_line.chars().count() > SNIPPET_MAX {
            one_line = one_line.chars().take(SNIPPET_MAX).collect::<String>() + "...";
        }
        format!("cuerpo: {one_line}")
    }
}

struct LoginFailure {
    retry: bool,
    message: String,
}

impl LoginFailure {
    fn transient(message: String) -> Self {
        Self {
            retry: true,
            message,
        }
    }

    fn fatal(message: String) -> Self {
        Self {
            retry: false,
            message,
        }
    }
}

/// Key/value pairs whose key names a secret (token, clave, password,
/// authorization...), JSON or form style.
static SECRET_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(\w*(?:token|clave|password|passwd|pwd|secret|authorization)\w*"?\s*[:=]\s*)(?:"(?:[^"\\]|\\.)+"|(?:bearer\s+)?[^\s",;}&]+)"#,
    )
    .unwrap_or_else(|_| unreachable!("constant regex"))
});

/// Anything shaped like a JWT, wherever it appears.
static JWT_LIKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"eyJ[A-Za-z0-9_-]*\.[A-Za-z0-9_-]+\.?[A-Za-z0-9_-]*")
        .unwrap_or_else(|_| unreachable!("constant regex"))
});

/// The error and its causes (reqwest hides the useful part in the sources),
/// without the URL.
fn error_chain(error: &reqwest::Error) -> String {
    let mut parts = vec![];
    let mut current: Option<&dyn std::error::Error> = Some(error);
    while let Some(e) = current {
        let text = e.to_string();
        if !parts.contains(&text) {
            parts.push(text);
        }
        current = e.source();
    }
    parts.join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_urls_are_scheme_host_port_only() {
        assert!(parse_base_url("https://host:9443").is_ok());
        assert!(parse_base_url(" http://127.0.0.1:8080/ ").is_ok());
        for bad in [
            "host:9443",
            "ftp://h",
            "https://h/api",
            "https://h?x=1",
            "",
            "https://",
        ] {
            assert!(parse_base_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn proceso_id_accepts_numbers_text_and_floats() {
        assert_eq!(parse_proceso_id(br#"{"ProcesoId":4242}"#), Some(4242));
        assert_eq!(parse_proceso_id(br#"{"ProcesoId":" 77 "}"#), Some(77));
        assert_eq!(parse_proceso_id(br#"{"ProcesoId":12.0}"#), Some(12));
        for bad in [
            &br#"{}"#[..],
            br#"{"ProcesoId":null}"#,
            br#"{"ProcesoId":"x"}"#,
            b"[]",
            b"",
        ] {
            assert_eq!(parse_proceso_id(bad), None);
        }
    }
}
