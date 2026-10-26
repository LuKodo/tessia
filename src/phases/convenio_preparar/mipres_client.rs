//! Read-only client of the MIPRES NO PBS delivery web service (port of Go
//! `adapters/mipres/client.go` plus the `mipresChecker` adapter of
//! `cmd/tessia/mipres_client.go`).
//!
//! ```text
//! GET {base}/api/GenerarToken/{nit}/{tokenPPAL}                    -> "secondary token"
//! GET {base}/api/EntregaXPrescripcion/{nit}/{token}/{prescription} -> [ {ID, IDEntrega, EstEntrega, ...} ]
//! ```
//!
//! The primary token (`mipres.token_ppal`) is traded for a secondary token
//! once per client; it lives in memory only and is regenerated once when the
//! service answers 401/403. Both tokens travel in the URL path, so URLs are
//! never logged and every error message is scrubbed of them.
//!
//! Every GET is retried on transport errors and 408/425/429/5xx (4 attempts,
//! 1 s / 2 s / 4 s, 30 s per attempt). Errors are classified for the rule
//! engine ([`MipresError`]):
//!
//! * rejected credentials (the token cannot be generated, or a second 401/403
//!   follows a regeneration) are **systemic** (`Abort`) and remembered, so a
//!   broken service is not hammered by every prescription;
//! * a persistent outage is `Unavailable`; the first one is remembered by the
//!   client for the whole run (the "breaker": no further request is made) but
//!   never counts as a credential failure;
//! * anything else is specific to one prescription.

use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde_json::Value;
use url::Url;

use super::mipres::{Delivery, MipresChecker, MipresError};
use crate::config::Settings;
use crate::http::{ClientOptions, RetryPolicy, build_client, send_with_retry};

const GENERATE_PATH: &str = "/api/GenerarToken/";
const DELIVERY_PATH: &str = "/api/EntregaXPrescripcion/";
const MAX_RESPONSE: u64 = 16 << 20;
const REDACTED: &str = "***";
/// Per-attempt timeout.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
/// Unknown or missing `EstEntrega`.
pub const STATE_UNKNOWN: i32 = -1;

/// Keys required whenever an EVENTO NO PBS invoice must be checked.
pub const REQUIRED_KEYS: [&str; 2] = ["mipres.nit", "mipres.token_ppal"];

/// Settings of the client.
#[derive(Clone)]
pub struct MipresOptions {
    /// `scheme://host[:port][/prefix]`.
    pub base_url: String,
    pub nit: String,
    /// The primary token (a secret).
    pub token_ppal: String,
    pub ca_file: Option<String>,
    /// Retry policy of every GET (tests use an instant one).
    pub retry: RetryPolicy,
}

impl std::fmt::Debug for MipresOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MipresOptions")
            .field("base_url", &self.base_url)
            .field("nit", &self.nit)
            .finish_non_exhaustive()
    }
}

impl MipresOptions {
    /// Reads `mipres.*`; the missing keys are named in the error.
    pub fn from_settings(settings: &Settings) -> Result<Self> {
        settings
            .require(&REQUIRED_KEYS)
            .context("EVENTO NO PBS invoices need the MIPRES delivery check")?;
        Ok(Self {
            base_url: settings.get_or("mipres.base_url", "").to_owned(),
            nit: settings.get_or("mipres.nit", "").to_owned(),
            token_ppal: settings.get_or("mipres.token_ppal", "").to_owned(),
            ca_file: settings.get("mipres.ca_file").map(str::to_owned),
            retry: production_retry(),
        })
    }
}

/// 4 attempts with 1 s, 2 s and 4 s backoffs.
pub fn production_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 4,
        base_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(10),
        ..RetryPolicy::default()
    }
}

#[derive(Default)]
struct TokenState {
    /// The secondary token (memory only).
    token: Option<String>,
    /// Systemic failure, remembered for the life of the client.
    failed: Option<MipresError>,
}

/// The MIPRES client; also the [`MipresChecker`] of a run. Safe to share.
pub struct MipresClient {
    http: Client,
    base: String,
    nit: String,
    token_ppal: String,
    retry: RetryPolicy,
    /// Held while a token is generated so concurrent callers share one call.
    state: Mutex<TokenState>,
    /// The first outage; once set the service is not asked again.
    down: Mutex<Option<MipresError>>,
}

impl MipresClient {
    /// Validates the options and builds the HTTP client.
    pub fn new(options: MipresOptions) -> Result<Self> {
        let base = options.base_url.trim().trim_end_matches('/').to_owned();
        let valid = Url::parse(&base).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.host_str().is_some()
                && u.query().is_none()
                && u.fragment().is_none()
        });
        if !valid {
            bail!("mipres: base URL must be scheme://host[/prefix] (mipres.base_url)");
        }
        if options.nit.trim().is_empty() || options.token_ppal.trim().is_empty() {
            bail!("mipres: NIT and primary token are required (mipres.nit, mipres.token_ppal)");
        }
        let http = build_client(
            &ClientOptions::default()
                .with_timeout(ATTEMPT_TIMEOUT)
                .with_ca_file(options.ca_file.as_deref()),
        )?;
        Ok(Self {
            http,
            base,
            nit: options.nit,
            token_ppal: options.token_ppal,
            retry: options.retry,
            state: Mutex::new(TokenState::default()),
            down: Mutex::new(None),
        })
    }

    /// Generates the secondary token if there is none yet, so a command can
    /// fail fast before any heavy work. A rejected credential is
    /// `Err(Abort)`; an outage is `Err(Unavailable)` (the caller warns and
    /// continues: the invoices that need MIPRES are held for the next run).
    pub fn authenticate(&self) -> Result<(), MipresError> {
        self.secondary().map(|_| ()).map_err(|e| self.classify(e))
    }

    /// Always performs the token request.
    pub fn generate_token(&self) -> Result<String, MipresError> {
        let url = format!(
            "{}{GENERATE_PATH}{}/{}",
            self.base,
            path_escape(&self.nit),
            path_escape(&self.token_ppal)
        );
        let (raw, status) = self.get(&url, "token request", &[])?;
        if status != 200 {
            return Err(MipresError::Other(format!("mipres: HTTP {status}")));
        }
        // The service answers a JSON string; bare text, objects and numbers
        // are refused.
        let token = serde_json::from_slice::<Option<String>>(&raw).map_err(|_| {
            MipresError::Other("mipres: token response is not a JSON string".into())
        })?;
        let token = token.unwrap_or_default().trim().to_owned();
        if token.is_empty() {
            return Err(MipresError::Other(
                "mipres: empty token in the token response".into(),
            ));
        }
        Ok(token)
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, TokenState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The current secondary token, generated on first use.
    fn secondary(&self) -> Result<String, MipresError> {
        let mut state = self.lock_state();
        if let Some(failed) = &state.failed {
            return Err(failed.clone());
        }
        if let Some(token) = &state.token {
            return Ok(token.clone());
        }
        self.generate_locked(&mut state, "generating MIPRES secondary token")
    }

    /// Replaces the token the service rejected; when another caller already
    /// did, the current one is returned (concurrent rejections regenerate once).
    fn regenerate(&self, stale: &str) -> Result<String, MipresError> {
        let mut state = self.lock_state();
        if let Some(failed) = &state.failed {
            return Err(failed.clone());
        }
        if let Some(current) = state.token.clone().filter(|t| t != stale) {
            return Ok(current);
        }
        self.generate_locked(&mut state, "regenerating MIPRES secondary token")
    }

    fn generate_locked(&self, state: &mut TokenState, step: &str) -> Result<String, MipresError> {
        match self.generate_token() {
            Ok(token) => {
                state.token = Some(token.clone());
                Ok(token)
            }
            Err(MipresError::Unavailable(cause)) => {
                // Transient: reported to this caller only; the next one tries again.
                Err(MipresError::Unavailable(step_error(step, &cause)))
            }
            Err(error) => {
                let failed = MipresError::Abort(step_error(step, error.detail()));
                state.failed = Some(failed.clone());
                Err(failed)
            }
        }
    }

    /// Remembers a systemic failure for the rest of the client's life.
    fn fail(&self, error: MipresError) -> MipresError {
        let mut state = self.lock_state();
        state.failed.get_or_insert(error).clone()
    }

    /// Lists the deliveries of one prescription (Go `Client.Entregas`). A
    /// 401/403 means the secondary token expired: it is regenerated once and
    /// the call retried once; a second rejection is systemic.
    pub fn list(&self, prescription: &str) -> Result<Vec<Delivery>, MipresError> {
        let prescription = prescription.trim();
        if prescription.is_empty() || prescription == "." || prescription == ".." {
            return Err(MipresError::Other(
                "mipres: invalid prescription number".into(),
            ));
        }
        let token = self.secondary()?;
        let (rows, status) = self.deliveries(&token, prescription);
        if !auth_rejected(status) {
            return rows;
        }
        let token = self.regenerate(&token)?;
        let (rows, status) = self.deliveries(&token, prescription);
        if auth_rejected(status) {
            let cause = rows.err().map_or_else(
                || "mipres: HTTP 401/403".to_owned(),
                |e| e.detail().to_owned(),
            );
            return Err(self.fail(MipresError::Abort(step_error(
                "listing entregas after regenerating MIPRES secondary token",
                &cause,
            ))));
        }
        rows
    }

    /// One `EntregaXPrescripcion` request; `status` is the HTTP status when
    /// the service answered, 0 otherwise.
    fn deliveries(
        &self,
        token: &str,
        prescription: &str,
    ) -> (Result<Vec<Delivery>, MipresError>, u16) {
        let url = format!(
            "{}{DELIVERY_PATH}{}/{}/{}",
            self.base,
            path_escape(&self.nit),
            path_escape(token),
            path_escape(prescription)
        );
        let (raw, status) = match self.get(&url, "entregas request", &[token]) {
            Ok(answer) => answer,
            Err(error) => return (Err(error), 0),
        };
        if status != 200 {
            return (
                Err(MipresError::Other(format!("mipres: HTTP {status}"))),
                status,
            );
        }
        (decode(&raw), status)
    }

    /// Performs a GET; returns the body and status. Errors are scrubbed of the
    /// primary token and of `extra` (the secondary token in use).
    fn get(&self, url: &str, what: &str, extra: &[&str]) -> Result<(Vec<u8>, u16), MipresError> {
        if crate::shutdown::requested() {
            return Err(MipresError::Abort(format!("mipres: {what}: interrumpido")));
        }
        let unavailable =
            |cause: String| MipresError::Unavailable(self.scrub(&step_error(what, &cause), extra));
        let response = send_with_retry(&self.retry, None, || {
            self.http.get(url).header("Accept", "application/json")
        })
        .map_err(|error| unavailable(transport_cause(&error)))?;
        let status = response.status().as_u16();
        if matches!(status, 408 | 425 | 429 | 500..=599) {
            return Err(unavailable(format!("HTTP {status}")));
        }
        let mut body = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(response, MAX_RESPONSE), &mut body)
            .map_err(|error| {
                MipresError::Other(self.scrub(&format!("mipres: read response: {error}"), extra))
            })?;
        Ok((body, status))
    }

    /// Hides the primary token and every extra secret (raw and escaped).
    fn scrub(&self, message: &str, extra: &[&str]) -> String {
        let mut out = message.to_owned();
        for secret in std::iter::once(self.token_ppal.as_str()).chain(extra.iter().copied()) {
            if secret.is_empty() {
                continue;
            }
            out = out.replace(secret, REDACTED);
            out = out.replace(&path_escape(secret), REDACTED);
        }
        out
    }

    /// The run-level breaker: the first outage is remembered and returned for
    /// every later call without another request.
    fn classify(&self, error: MipresError) -> MipresError {
        if !matches!(error, MipresError::Unavailable(_)) {
            return error;
        }
        let mut down = self.down.lock().unwrap_or_else(|e| e.into_inner());
        down.get_or_insert(error).clone()
    }

    fn breaker(&self) -> Option<MipresError> {
        self.down.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl MipresChecker for MipresClient {
    fn entregas(&self, prescription: &str) -> Result<Vec<Delivery>, MipresError> {
        if let Some(down) = self.breaker() {
            return Err(down);
        }
        self.list(prescription).map_err(|e| self.classify(e))
    }
}

impl MipresError {
    /// The message without the kind prefix.
    pub fn detail(&self) -> &str {
        match self {
            MipresError::Abort(m) | MipresError::Unavailable(m) | MipresError::Other(m) => m,
        }
    }
}

fn auth_rejected(status: u16) -> bool {
    status == 401 || status == 403
}

/// `mipres: <step>: <cause>` with the cause's own `mipres: ` prefix dropped.
fn step_error(step: &str, cause: &str) -> String {
    format!(
        "mipres: {step}: {}",
        cause.strip_prefix("mipres: ").unwrap_or(cause)
    )
}

fn transport_cause(error: &crate::http::HttpError) -> String {
    match error {
        crate::http::HttpError::Transport { source, .. } => source.to_string(),
        other => other.to_string(),
    }
}

/// Go `url.PathEscape`: everything but unreserved characters and
/// `$ & + = : @` is percent-encoded.
pub fn path_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'$'
            | b'&'
            | b'+'
            | b'='
            | b':'
            | b'@' => out.push(char::from(byte)),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Tolerant decoding of the delivery list: `ID`, `IDEntrega` and `EstEntrega`
/// arrive as numbers or strings, keys match case-insensitively, unknown fields
/// are ignored; a blank or `null` answer is an empty list.
fn decode(raw: &[u8]) -> Result<Vec<Delivery>, MipresError> {
    let not_a_list =
        || MipresError::Other("mipres: response is not a JSON array of deliveries".into());
    if String::from_utf8_lossy(raw).trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_slice(raw).map_err(|_| not_a_list())?;
    let rows = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(rows) => rows,
        _ => return Err(not_a_list()),
    };
    rows.iter()
        .map(|row| {
            let object = row.as_object().ok_or_else(not_a_list)?;
            let field = |name: &str| {
                object
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map_or_else(String::new, |(_, value)| flex(value))
            };
            Ok(Delivery {
                id: field("ID"),
                delivery_id: field("IDEntrega"),
                state: parse_state(&field("EstEntrega")),
            })
        })
        .collect()
}

/// A JSON string (trimmed) or the raw text of any other value; `null` is empty.
fn flex(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.trim().to_owned(),
        other => other.to_string(),
    }
}

fn parse_state(text: &str) -> i32 {
    if let Ok(n) = text.parse::<i32>() {
        return n;
    }
    match text.parse::<f64>() {
        Ok(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < f64::from(i32::MAX) => f as i32,
        _ => STATE_UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::testing::{MockResponse, MockServer};

    const NIT: &str = "900123456";
    const PPAL: &str = "primary-secret-ppal";
    const TOKEN: &str = "secondary-secret-token";

    fn options(server: &MockServer) -> MipresOptions {
        MipresOptions {
            base_url: server.url().to_owned(),
            nit: NIT.into(),
            token_ppal: PPAL.into(),
            ca_file: None,
            retry: RetryPolicy::instant(4),
        }
    }

    fn client(server: &MockServer) -> MipresClient {
        MipresClient::new(options(server)).expect("client")
    }

    fn routes(
        deliveries: &'static str,
    ) -> impl Fn(&crate::testing::RecordedRequest) -> MockResponse + Send + 'static {
        move |request| {
            if request.path.starts_with("/api/GenerarToken/") {
                MockResponse::json(200, &format!("\"{TOKEN}\""))
            } else if request.path.starts_with("/api/EntregaXPrescripcion/") {
                MockResponse::json(200, deliveries)
            } else {
                MockResponse::text(404, "")
            }
        }
    }

    fn paths(server: &MockServer) -> Vec<String> {
        server.requests().into_iter().map(|r| r.path).collect()
    }

    #[test]
    fn deliveries_decode_tolerantly() {
        let server = MockServer::start(routes(
            r#"[
            {"ID": 123, "IDEntrega": 9, "NoPrescripcion": "20240101", "EstEntrega": 2, "Otro": {"x": 1}},
            {"id": "456", "estentrega": "0"},
            {"ID": 789, "EstEntrega": null},
            {"ID": 790},
            {"ID": " 5 ", "EstEntrega": 1.0}
        ]"#,
        ));
        let got = client(&server).entregas("20240101").expect("deliveries");
        let expected: Vec<(&str, &str, i32)> = vec![
            ("123", "9", 2),
            ("456", "", 0),
            ("789", "", -1),
            ("790", "", -1),
            ("5", "", 1),
        ];
        let got: Vec<(&str, &str, i32)> = got
            .iter()
            .map(|d| (d.id.as_str(), d.delivery_id.as_str(), d.state))
            .collect();
        assert_eq!(got, expected);
        let requests = server.requests();
        assert_eq!(requests.len(), 2, "token + one delivery request");
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, format!("/api/GenerarToken/{NIT}/{PPAL}"));
        assert_eq!(
            requests[1].path,
            format!("/api/EntregaXPrescripcion/{NIT}/{TOKEN}/20240101")
        );
        assert_eq!(requests[1].header("accept"), Some("application/json"));
    }

    #[test]
    fn blank_null_and_empty_answers_are_empty_lists() {
        for body in ["[]", "null", "", "  "] {
            let server = MockServer::start(routes(Box::leak(body.to_owned().into_boxed_str())));
            assert!(
                client(&server).entregas("1").expect("ok").is_empty(),
                "{body:?}"
            );
        }
    }

    #[test]
    fn every_path_segment_is_escaped_and_the_base_may_carry_a_prefix() {
        let server = MockServer::start(|request| {
            if request.path.contains("/api/GenerarToken/") {
                MockResponse::json(200, "\"d/e f\"")
            } else {
                MockResponse::json(200, "[]")
            }
        });
        let mut opts = options(&server);
        opts.base_url = format!("{}/WSSUMMIPRESNOPBS/", server.url());
        opts.nit = "9/0 0".into();
        opts.token_ppal = "a?b#c".into();
        let client = MipresClient::new(opts).expect("client");
        client.entregas("A/B 1").expect("ok");
        assert_eq!(
            paths(&server),
            [
                "/WSSUMMIPRESNOPBS/api/GenerarToken/9%2F0%200/a%3Fb%23c",
                "/WSSUMMIPRESNOPBS/api/EntregaXPrescripcion/9%2F0%200/d%2Fe%20f/A%2FB%201",
            ]
        );
    }

    #[test]
    fn bad_prescription_numbers_and_options_are_rejected() {
        let server = MockServer::start(routes("[]"));
        let c = client(&server);
        for bad in ["", "  ", ".", ".."] {
            assert_eq!(
                c.entregas(bad).expect_err("bad"),
                MipresError::Other("mipres: invalid prescription number".into())
            );
        }
        assert!(server.requests().is_empty());
        for (base, nit, ppal) in [
            ("", NIT, PPAL),
            ("ftp://x", NIT, PPAL),
            ("http://x?q=1", NIT, PPAL),
            ("http://x#f", NIT, PPAL),
            ("http://x", "", PPAL),
            ("http://x", NIT, " "),
        ] {
            let mut opts = options(&server);
            (opts.base_url, opts.nit, opts.token_ppal) = (base.into(), nit.into(), ppal.into());
            assert!(
                MipresClient::new(opts).is_err(),
                "{base:?} {nit:?} {ppal:?}"
            );
        }
    }

    #[test]
    fn an_http_error_mentions_only_the_status() {
        let server = MockServer::start(|request| {
            if request.path.starts_with("/api/GenerarToken/") {
                MockResponse::json(200, &format!("\"{TOKEN}\""))
            } else {
                MockResponse::json(404, &format!("{{\"Message\":\"token {TOKEN} invalido\"}}"))
            }
        });
        let error = client(&server).entregas("1").expect_err("404");
        assert_eq!(error, MipresError::Other("mipres: HTTP 404".into()));
    }

    #[test]
    fn a_bad_body_does_not_echo_it() {
        let server = MockServer::start(routes(Box::leak(
            format!("{{\"Message\":\"{TOKEN}\"}}").into_boxed_str(),
        )));
        let error = client(&server).entregas("1").expect_err("not a list");
        assert_eq!(
            error.to_string(),
            "mipres: response is not a JSON array of deliveries"
        );
    }

    #[test]
    fn the_token_must_be_a_json_string() {
        let cases: [(u16, &str); 9] = [
            (200, "\"\""),
            (200, "\"   \""),
            (200, ""),
            (200, "null"),
            (200, "{\"token\":\"x\"}"),
            (200, "12345"),
            (200, "bare-text"),
            (200, "\"truncated"),
            (401, "\"x\""),
        ];
        for (status, body) in cases {
            let server = MockServer::start(move |_| MockResponse::json(status, body));
            let c = client(&server);
            let error = c.generate_token().expect_err(body);
            let message = error.to_string();
            assert!(!message.contains(PPAL), "{message}");
            if status != 200 {
                assert!(message.contains("401"), "{message}");
            }
        }
        let server = MockServer::start(|_| MockResponse::json(200, "  \"  tok-1 \"\n"));
        assert_eq!(client(&server).generate_token().expect("token"), "tok-1");
    }

    #[test]
    fn the_token_is_generated_once_per_client() {
        let server = MockServer::start(routes("[]"));
        let c = client(&server);
        c.authenticate().expect("auth");
        c.authenticate().expect("auth again");
        c.entregas("1").expect("a");
        c.entregas("2").expect("b");
        let generations = paths(&server)
            .iter()
            .filter(|p| p.starts_with("/api/GenerarToken/"))
            .count();
        assert_eq!(generations, 1);
        assert_eq!(server.requests().len(), 3);
    }

    #[test]
    fn a_failed_generation_is_systemic_remembered_and_not_retried() {
        let server = MockServer::start(|_| MockResponse::json(401, "{}"));
        let c = client(&server);
        let first = c.authenticate().expect_err("rejected");
        assert!(matches!(first, MipresError::Abort(_)), "{first:?}");
        assert_eq!(
            first.to_string(),
            "MIPRES check aborted: mipres: generating MIPRES secondary token: HTTP 401"
        );
        for _ in 0..3 {
            assert_eq!(c.entregas("1").expect_err("remembered"), first);
            assert_eq!(c.authenticate().expect_err("remembered"), first);
        }
        assert_eq!(
            server.requests().len(),
            1,
            "exactly the one failed generation"
        );
    }

    #[test]
    fn a_rejected_token_is_regenerated_once_and_the_call_retried() {
        let tokens = Arc::new(Mutex::new(0usize));
        let counter = tokens.clone();
        let server = MockServer::start(move |request| {
            if request.path.starts_with("/api/GenerarToken/") {
                let mut n = counter.lock().expect("counter");
                *n += 1;
                return MockResponse::json(200, &format!("\"tok-{n}\""));
            }
            if request.path.contains("/tok-1/") {
                MockResponse::json(401, "{}")
            } else {
                MockResponse::json(200, r#"[{"ID":"7","IDEntrega":"77","EstEntrega":2}]"#)
            }
        });
        let got = client(&server).entregas("1").expect("retried");
        assert_eq!(
            got,
            vec![Delivery {
                id: "7".into(),
                delivery_id: "77".into(),
                state: 2
            }]
        );
        assert_eq!(*tokens.lock().expect("counter"), 2);
        assert_eq!(server.requests().len(), 4, "token, 401, token, ok");
    }

    #[test]
    fn a_persistent_rejection_is_systemic_without_secrets() {
        let server = MockServer::start(|request| {
            if request.path.starts_with("/api/GenerarToken/") {
                MockResponse::json(200, &format!("\"{TOKEN}\""))
            } else {
                MockResponse::json(403, &format!("{{\"Message\":\"{TOKEN}\"}}"))
            }
        });
        let c = client(&server);
        let error = c.entregas("1").expect_err("403");
        assert!(matches!(error, MipresError::Abort(_)));
        let message = error.to_string();
        assert!(message.contains("403"), "{message}");
        assert!(
            !message.contains(TOKEN) && !message.contains(PPAL),
            "{message}"
        );
        let before = server.requests().len();
        assert_eq!(c.entregas("2").expect_err("remembered"), error);
        assert_eq!(
            server.requests().len(),
            before,
            "no request after the rejection"
        );
    }

    #[test]
    fn other_statuses_do_not_regenerate_the_token() {
        for status in [400u16, 404, 422] {
            let server = MockServer::start(move |request| {
                if request.path.starts_with("/api/GenerarToken/") {
                    MockResponse::json(200, &format!("\"{TOKEN}\""))
                } else {
                    MockResponse::json(status, "{}")
                }
            });
            let error = client(&server).entregas("1").expect_err("status");
            assert_eq!(error, MipresError::Other(format!("mipres: HTTP {status}")));
            assert_eq!(server.requests().len(), 2, "no regeneration for {status}");
        }
    }

    #[test]
    fn server_errors_are_retried_until_the_answer_arrives() {
        let server = MockServer::sequence(vec![
            MockResponse::text(503, ""),
            MockResponse::text(500, ""),
            MockResponse::text(429, ""),
            MockResponse::json(200, &format!("\"{TOKEN}\"")),
        ]);
        assert_eq!(client(&server).generate_token().expect("token"), TOKEN);
        assert_eq!(server.requests().len(), 4);
    }

    #[test]
    fn a_persistent_outage_is_unavailable_not_auth_and_trips_the_breaker() {
        let server = MockServer::start(|_| MockResponse::text(503, "down"));
        let c = client(&server);
        let error = c.entregas("1").expect_err("outage");
        assert!(matches!(error, MipresError::Unavailable(_)), "{error:?}");
        let message = error.to_string();
        assert!(message.contains("503") && message.starts_with("MIPRES unavailable: mipres: "));
        assert!(!message.contains(PPAL));
        assert_eq!(
            server.requests().len(),
            4,
            "one retry sequence of 4 attempts"
        );
        // The breaker: later prescriptions get the same error without a request.
        assert_eq!(c.entregas("2").expect_err("breaker"), error);
        assert_eq!(server.requests().len(), 4);
    }

    #[test]
    fn an_outage_never_poisons_the_client_for_direct_calls() {
        let server = MockServer::sequence(vec![
            MockResponse::text(503, ""),
            MockResponse::text(503, ""),
            MockResponse::text(503, ""),
            MockResponse::text(503, ""),
            MockResponse::json(200, &format!("\"{TOKEN}\"")),
            MockResponse::json(200, "[]"),
        ]);
        let c = client(&server);
        assert!(matches!(c.list("1"), Err(MipresError::Unavailable(_))));
        assert_eq!(c.list("1").expect("recovered"), vec![]);
    }

    #[test]
    fn a_transport_failure_is_unavailable_and_leaks_no_secret() {
        // Nothing listens on the port: connection refused on every attempt.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr").port()
        };
        let mut opts = MipresOptions {
            base_url: format!("http://127.0.0.1:{port}"),
            nit: NIT.into(),
            token_ppal: PPAL.into(),
            ca_file: None,
            retry: RetryPolicy::instant(2),
        };
        opts.retry.jitter = 0.0;
        let error = MipresClient::new(opts)
            .expect("client")
            .entregas("1")
            .expect_err("refused");
        assert!(matches!(error, MipresError::Unavailable(_)), "{error:?}");
        let message = error.to_string();
        assert!(
            !message.contains(PPAL) && !message.contains("/api/"),
            "{message}"
        );
    }

    #[test]
    fn scrubbing_hides_raw_and_escaped_secrets() {
        let server = MockServer::start(routes("[]"));
        let mut opts = options(&server);
        opts.token_ppal = "a b/c".into();
        let c = MipresClient::new(opts).expect("client");
        let scrubbed = c.scrub("x a b/c y a%20b%2Fc z tok", &["tok"]);
        assert_eq!(scrubbed, "x *** y *** z ***");
    }

    #[test]
    fn path_escape_matches_go() {
        assert_eq!(path_escape("9/0 0"), "9%2F0%200");
        assert_eq!(path_escape("a?b#c"), "a%3Fb%23c");
        assert_eq!(path_escape("d/e f"), "d%2Fe%20f");
        assert_eq!(path_escape("x$&+=:@-_.~"), "x$&+=:@-_.~");
        assert_eq!(path_escape("ñ;,"), "%C3%B1%3B%2C");
    }

    #[test]
    fn options_come_from_the_settings() {
        let settings = Settings::from_pairs([
            ("TESSIA_MIPRES_NIT", NIT),
            ("TESSIA_MIPRES_TOKEN_PPAL", PPAL),
        ]);
        let options = MipresOptions::from_settings(&settings).expect("options");
        assert_eq!(options.nit, NIT);
        assert!(
            options.base_url.starts_with("https://wsmipres"),
            "default base URL"
        );
        let missing =
            MipresOptions::from_settings(&Settings::from_pairs([("TESSIA_MIPRES_NIT", NIT)]))
                .expect_err("token missing");
        let text = format!("{missing:#}");
        assert!(text.contains("EVENTO NO PBS invoices need the MIPRES delivery check"));
        assert!(
            text.contains("mipres.token_ppal") && !text.contains(PPAL),
            "{text}"
        );
    }
}
