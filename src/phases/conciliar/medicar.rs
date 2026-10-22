//! The Medicar formula-date client (CONC-07, CONC-08), a port of Go
//! `internal/adapters/medicar/formula.go`.
//!
//! One [`FormulaClient`] has its own session: login once (`GET /` for the
//! `csrf_token`, then `POST /procesar.php` with the MD5 user and password), the
//! formula list once per patient document (`POST /modalAjax/Control.php`) and
//! the formula detail (`POST /modalAjax/VerFormula.php`). Every request is a
//! read, so each one (login included) makes up to 4 attempts with 1, 2 and 4
//! second backoffs on transport errors and 408/425/429/5xx; the POST bodies are
//! sent again each time.
//!
//! A portal that still fails after that is *unavailable*
//! ([`MedicarError::Unavailable`]): transient, the invoice stays `FAILED` and
//! the next run retries it. A rejected login (4xx), a missing csrf token, a
//! formula Medicar does not relate to the invoice, a missing molecule and an
//! unparseable page are *permanent* (`REVISION_MANUAL`).
//!
//! The first unavailable answer is remembered in an [`Outage`] for the rest of
//! the command (or of the `proceso run`, when clients share one): later lookups
//! return the same error without a request.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::Result;
use md5::{Digest, Md5};
use reqwest::blocking::{Client, RequestBuilder};
use thiserror::Error;
use url::form_urlencoded;

use super::portal_html::{
    Formula, decode_portal, find_csrf, list_contains_formula, parse_formula_detail,
};
use crate::config::Settings;
use crate::http::{
    ClientOptions, HttpError, MEDICAR_USER_AGENT, RetryPolicy, build_client, send_with_retry,
};

/// Per-request timeout of formula lookups (PHP: 45 s).
pub const DEFAULT_FORMULA_TIMEOUT: Duration = Duration::from_secs(45);

/// Attempts per request (login and lookups).
pub const REQUEST_ATTEMPTS: u32 = 4;

/// Largest page read (Go: 16 MiB).
const MAX_BODY: u64 = 16 << 20;

/// Why a lookup failed.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum MedicarError {
    /// Retrying cannot fix it: bad input, a rejected login, a formula Medicar
    /// does not relate to the invoice, an unparseable page, HTTP 4xx.
    #[error("{0}")]
    Permanent(String),
    /// The portal could not be reached or answered 408/425/429/5xx even after
    /// the retries. Transient: worth retrying in a later run.
    #[error("{0}")]
    Unavailable(String),
    /// Any other failure (reading a body): neither permanent nor an outage.
    #[error("{0}")]
    Transient(String),
}

impl MedicarError {
    /// True for [`MedicarError::Permanent`].
    pub fn is_permanent(&self) -> bool {
        matches!(self, MedicarError::Permanent(_))
    }

    /// True for [`MedicarError::Unavailable`].
    pub fn is_unavailable(&self) -> bool {
        matches!(self, MedicarError::Unavailable(_))
    }
}

fn permanent(msg: impl std::fmt::Display) -> MedicarError {
    MedicarError::Permanent(format!("medicar: {msg}"))
}

/// Remembers the first [`MedicarError::Unavailable`] seen: while it is set, a
/// [`FormulaClient`] sends no request and returns it, so the pending invoices
/// of a command do not each wait their own retry sequence for a portal that is
/// down. Safe for concurrent use; nothing clears it.
#[derive(Debug, Default)]
pub struct Outage {
    err: Mutex<Option<MedicarError>>,
}

impl Outage {
    pub fn new() -> Self {
        Self::default()
    }

    fn slot(&self) -> MutexGuard<'_, Option<MedicarError>> {
        // A poisoned lock only means another thread panicked; the value is fine.
        self.err.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The remembered outage, if any.
    pub fn err(&self) -> Option<MedicarError> {
        self.slot().clone()
    }

    /// Remembers `err` when it is an outage and returns the error to report:
    /// the outage remembered first (all callers see the same one) or `err`.
    pub fn note(&self, err: MedicarError) -> MedicarError {
        if !err.is_unavailable() {
            return err;
        }
        self.slot().get_or_insert(err).clone()
    }
}

/// Client settings.
#[derive(Clone)]
pub struct MedicarOptions {
    pub base_url: String,
    pub identifier: String,
    pub username: String,
    pub password: String,
    /// Optional PEM CA bundle (`medicar.ca_file`).
    pub ca_file: Option<String>,
    /// Per-request timeout.
    pub read_timeout: Duration,
    /// Retry policy of every request; tests replace the sleeper.
    pub retry: RetryPolicy,
    /// Shared outage memory; `None` = a private one per client.
    pub outage: Option<Arc<Outage>>,
}

impl std::fmt::Debug for MedicarOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print credentials.
        f.debug_struct("MedicarOptions")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

/// 4 attempts, 1 s doubling up to 10 s, POSTs retried too.
pub fn formula_retry_policy() -> RetryPolicy {
    RetryPolicy {
        max_attempts: REQUEST_ATTEMPTS,
        base_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(10),
        retry_non_idempotent: true,
        ..RetryPolicy::default()
    }
}

impl MedicarOptions {
    pub fn new(base_url: &str, identifier: &str, username: &str, password: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            identifier: identifier.to_owned(),
            username: username.to_owned(),
            password: password.to_owned(),
            ca_file: None,
            read_timeout: DEFAULT_FORMULA_TIMEOUT,
            retry: formula_retry_policy(),
            outage: None,
        }
    }

    /// Reads `medicar.base_url|identifier|username|password` (and `ca_file`);
    /// an error names the missing keys and their environment variables.
    pub fn from_settings(settings: &Settings) -> Result<Self> {
        let [base, id, user, pass] =
            ["base_url", "identifier", "username", "password"].map(|k| format!("medicar.{k}"));
        settings.require(&[base.as_str(), id.as_str(), user.as_str(), pass.as_str()])?;
        let get = |k: &str| settings.get(k).unwrap_or_default().to_owned();
        let mut options = Self::new(&get(&base), &get(&id), &get(&user), &get(&pass));
        options.ca_file = settings.get("medicar.ca_file").map(str::to_owned);
        Ok(options)
    }
}

/// Session state guarded by one mutex (the Go client serializes lookups too).
#[derive(Default)]
struct Session {
    http: Option<Client>,
    /// Formula list page per patient document.
    lists: HashMap<String, String>,
}

/// Looks formulas up in the Medicar portal.
pub struct FormulaClient {
    opts: MedicarOptions,
    outage: Arc<Outage>,
    session: Mutex<Session>,
}

fn md5_hex(s: &str) -> String {
    // The portal expects MD5 hex digests; not a security choice of ours.
    hex::encode(Md5::digest(s.as_bytes()))
}

fn transient_status(code: u16) -> bool {
    matches!(code, 408 | 425 | 429) || code >= 500
}

/// The deepest cause of a transport error: no URL, no secrets.
fn root_cause(err: &reqwest::Error) -> String {
    let mut cause: &dyn std::error::Error = err;
    while let Some(next) = cause.source() {
        cause = next;
    }
    cause.to_string()
}

/// A request to send: method, path below the base URL and optional form body.
struct Call<'a> {
    what: &'a str,
    path: &'a str,
    form: Option<String>,
    accept: &'a str,
    referer: Option<String>,
}

impl FormulaClient {
    /// Validates the options.
    pub fn new(opts: MedicarOptions) -> Result<Self, MedicarError> {
        if opts.base_url.is_empty()
            || opts.identifier.is_empty()
            || opts.username.is_empty()
            || opts.password.is_empty()
        {
            return Err(MedicarError::Permanent(
                "medicar: base URL, identifier, username and password are required".into(),
            ));
        }
        let outage = opts.outage.clone().unwrap_or_default();
        Ok(Self {
            opts,
            outage,
            session: Mutex::new(Session::default()),
        })
    }

    /// The formula `ssc` of `document` after checking that Medicar relates it to
    /// `invoice`, choosing the detail row whose molecule best matches `article`.
    pub fn lookup(
        &self,
        document: &str,
        ssc: &str,
        invoice: &str,
        article: &str,
    ) -> Result<Formula, MedicarError> {
        let document: String = document.chars().filter(char::is_ascii_digit).collect();
        let ssc = ssc.trim();
        let invoice = invoice.trim().to_uppercase();
        if document.is_empty() || ssc.is_empty() || invoice.is_empty() || article.trim().is_empty()
        {
            return Err(permanent(
                "documento, SSC, factura y articulo son obligatorios para consultar Medicar",
            ));
        }
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let client = self.authenticate(&mut session)?;
        let list = match session.lists.get(&document) {
            Some(list) => list.clone(),
            None => {
                let list = self.post(&client, "/modalAjax/Control.php", &list_fields(&document))?;
                session.lists.insert(document.clone(), list.clone());
                list
            }
        };
        if !list_contains_formula(&list, ssc, &invoice) {
            return Err(permanent(format!(
                "Medicar no relaciona la formula {ssc} con la factura {invoice} para el documento consultado"
            )));
        }
        let detail = self.post(&client, "/modalAjax/VerFormula.php", &[("IdFormula", ssc)])?;
        let mut out = parse_formula_detail(&detail, ssc, article).map_err(|e| permanent(e.0))?;
        out.documento = document;
        out.factura = invoice;
        out.ssc = ssc.to_owned();
        Ok(out)
    }

    /// Opens the session. Once the portal is known to be down (this client or
    /// any sharing its [`Outage`]) it returns that outage without a request.
    fn authenticate(&self, session: &mut Session) -> Result<Client, MedicarError> {
        if let Some(client) = &session.http {
            return Ok(client.clone());
        }
        if let Some(err) = self.outage.err() {
            return Err(err);
        }
        let options = ClientOptions::default()
            .with_cookies()
            .with_user_agent(MEDICAR_USER_AGENT)
            .with_timeout(self.opts.read_timeout + Duration::from_secs(15))
            .with_ca_file(self.opts.ca_file.as_deref());
        let client = build_client(&options).map_err(|e| MedicarError::Permanent(e.to_string()))?;
        let home = Call {
            what: "GET /",
            path: "/",
            form: None,
            accept: "text/plain, */*; q=0.01",
            referer: None,
        };
        let (_, raw) = self.send_checked(&client, reqwest::Method::GET, &home)?;
        let page = decode_portal(&raw);
        let csrf = find_csrf(&page)
            .ok_or_else(|| permanent("Medicar no entrego el token de inicio de sesion"))?;
        self.login(&client, &csrf)
            .map_err(|e| self.outage.note(e))?;
        session.http = Some(client.clone());
        Ok(client)
    }

    fn login(&self, client: &Client, csrf: &str) -> Result<(), MedicarError> {
        let o = &self.opts;
        let form = form_urlencoded::Serializer::new(String::new())
            .append_pair("f_id", &o.identifier)
            .append_pair("f_log", &md5_hex(&o.username))
            .append_pair("f_pass", &md5_hex(&o.password))
            .append_pair("f_pass_nuevo", "")
            .append_pair("f_pass_confirma", "")
            .append_pair("csrf_token", csrf)
            .finish();
        let call = Call {
            what: "login",
            path: "/procesar.php",
            form: Some(form),
            accept: "application/json, text/javascript, */*; q=0.01",
            referer: Some(format!("{}/", o.base_url)),
        };
        let (status, raw) = self.exchange(client, reqwest::Method::POST, &call)?;
        if transient_status(status) {
            return Err(MedicarError::Unavailable(format!(
                "medicar: login: HTTP {status}"
            )));
        }
        if status != 200 || !String::from_utf8_lossy(&raw).contains("\"status\"") {
            return Err(permanent(format!("login was not accepted (HTTP {status})")));
        }
        Ok(())
    }

    fn post(
        &self,
        client: &Client,
        path: &str,
        fields: &[(&str, &str)],
    ) -> Result<String, MedicarError> {
        let mut form = form_urlencoded::Serializer::new(String::new());
        for (k, v) in fields {
            form.append_pair(k, v);
        }
        let what = format!("POST {path}");
        let call = Call {
            what: &what,
            path,
            form: Some(form.finish()),
            accept: "text/plain, */*; q=0.01",
            referer: Some(format!("{}/el_admin/control.php", self.opts.base_url)),
        };
        let (_, raw) = self.send_checked(client, reqwest::Method::POST, &call)?;
        Ok(decode_portal(&raw))
    }

    /// One request with the outage memory: while the portal is known to be
    /// down nothing is sent; an outage seen now is remembered. A status other
    /// than 200 is an error (transient statuses are an outage, 4xx permanent).
    fn send_checked(
        &self,
        client: &Client,
        method: reqwest::Method,
        call: &Call<'_>,
    ) -> Result<(u16, Vec<u8>), MedicarError> {
        if let Some(err) = self.outage.err() {
            return Err(err);
        }
        let result = self
            .exchange(client, method, call)
            .and_then(|(status, raw)| {
                if status == 200 {
                    return Ok((status, raw));
                }
                let msg = format!(
                    "medicar: la consulta a Medicar fallo ({}: HTTP {status})",
                    call.what
                );
                Err(if transient_status(status) {
                    MedicarError::Unavailable(msg)
                } else {
                    MedicarError::Permanent(msg)
                })
            });
        result.map_err(|e| self.outage.note(e))
    }

    /// Sends the call through the retry policy and returns the final status
    /// with the body. A transport failure is an outage with only the cause
    /// (never the URL).
    fn exchange(
        &self,
        client: &Client,
        method: reqwest::Method,
        call: &Call<'_>,
    ) -> Result<(u16, Vec<u8>), MedicarError> {
        let url = format!("{}{}", self.opts.base_url, call.path);
        let build = || self.request(client, method.clone(), &url, call);
        let response = send_with_retry(&self.opts.retry, None, build).map_err(|e| match e {
            HttpError::Transport { source, .. } => MedicarError::Unavailable(format!(
                "medicar: {}: {}",
                call.what,
                root_cause(&source)
            )),
            HttpError::Build(e) => MedicarError::Permanent(format!("medicar: {e}")),
        })?;
        let status = response.status().as_u16();
        let mut raw = Vec::new();
        response.take(MAX_BODY).read_to_end(&mut raw).map_err(|e| {
            MedicarError::Transient(format!("medicar: {}: read body: {e}", call.what))
        })?;
        Ok((status, raw))
    }

    fn request(
        &self,
        client: &Client,
        method: reqwest::Method,
        url: &str,
        call: &Call<'_>,
    ) -> RequestBuilder {
        let mut req = client
            .request(method, url)
            .header("Accept", call.accept)
            .header("X-Requested-With", "XMLHttpRequest");
        if let Some(referer) = &call.referer {
            req = req.header("Referer", referer);
        }
        match &call.form {
            Some(form) => req
                .header(
                    "Content-Type",
                    "application/x-www-form-urlencoded; charset=UTF-8",
                )
                .body(form.clone()),
            None => req,
        }
    }
}

fn list_fields(document: &str) -> [(&str, &str); 8] {
    [
        ("f_opciones", "3"),
        ("f_numero", document),
        ("f_tipo_formula", "Todos"),
        ("f_fecha", ""),
        ("f_estado", "Todos"),
        ("f_sucursal", "Todos"),
        ("f_perfil_punto", ""),
        ("f_splan", "Todos"),
    ]
}
