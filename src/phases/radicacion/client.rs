//! HTTP client of the Mutual Ser (ZonaSer benavente "rips-api") filing API (Go
//! `internal/adapters/mutualser` + `zonaser/tocumen`).
//!
//! All calls share one token source (refreshed on 401 and before expiry) and
//! one adaptive limiter (the process-wide `mutualser` one: starts at 20,
//! between 8 and 40), and every HTTP status is checked. Identity headers come
//! from configuration; nothing is hard-coded. Query strings, tokens and bodies
//! are never logged.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};
use reqwest::Method;
use reqwest::blocking::Client;
use serde_json::{Value, json};

use super::api::{ApiError, ApiResult, RadApi};
use super::model::{
    FileDetail, FiltersAllItem, Load, LoadFile, Message, SupportDocument, UploadFile, XmlDocument,
};
use super::portal::{
    Finding as PortalFinding, PortalApi, Record as PortalRecord, choose_record, parse_uploaded_at,
};
use crate::domain::radicacion::Step;
use crate::http::token::FetchedToken;
use crate::http::{
    AdaptiveLimiter, MUTUALSER_USER_AGENT, RetryPolicy, TokenCache, jwt_expiry, response_healthy,
    send_with_auth, send_with_retry,
};
use crate::phases::radicacion::timings::Timings;
use crate::util::par;

/// Mutual Ser document type ids (`id_tipo`) and file extensions per step.
fn step_type(step: Step) -> (&'static str, &'static str) {
    match step {
        Step::InvoiceZip => ("1da64d49-f5f7-4d70-8a7c-640db4816521", "zip"),
        Step::Cuv => ("5072926906777600", "txt"),
        Step::Rips => ("5643548095414272", "json"),
        Step::Crc => ("5655362556395520", "zip"),
    }
}

/// Largest API answer read.
const MAX_BODY: u64 = 32 << 20;
/// Heartbeat period for in-flight calls.
const SLOW_CALL_EVERY: Duration = Duration::from_secs(30);
/// Timeout of calls to ZonaSer (Go `radicacionProfile`).
pub const API_TIMEOUT: Duration = Duration::from_secs(150);
/// Timeout of the signed-URL uploads.
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// Tokens are refreshed this long before they expire.
const TOKEN_SKEW: Duration = Duration::from_secs(60);
/// Assumed token lifetime when the login says nothing and it is not a JWT.
const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(30 * 60);

/// Everything the client needs; the settings of `zonaser.*`.
#[derive(Clone)]
pub struct ClientConfig {
    /// The rips-api root, e.g. `https://host/mutual-api-rfds/api/v1/rips-api`.
    pub base_url: String,
    pub nit: String,
    pub organization_name: String,
    pub user_email: String,
    pub user_id: String,
    pub roles: Vec<String>,
    /// Sent as `origin`/`referer` (the web portal URL).
    pub portal_origin: String,
    pub login_url: String,
    pub username: String,
    pub password: String,
    /// Optional PEM bundle pinned for ZonaSer (never for the signed URLs).
    pub ca_file: Option<String>,
    pub api_timeout: Duration,
    pub upload_timeout: Duration,
    /// Policy of API calls (POSTs are retried only on 425/429).
    pub retry: RetryPolicy,
    /// Policy of the signed-URL PUTs (10 attempts, 1 s doubling to 2 min).
    pub upload_retry: RetryPolicy,
    /// File names per signed-URL request (avoids HTTP 414).
    pub signed_url_chunk: usize,
    /// Start of the `filtersAll` date window.
    pub filters_all_start: NaiveDate,
    /// Sign-of-life period of in-flight calls; `None` disables it.
    pub slow_call_every: Option<Duration>,
}

impl ClientConfig {
    /// The Go defaults for the given base URL, NIT and credentials.
    pub fn new(base_url: &str, nit: &str) -> ClientConfig {
        ClientConfig {
            base_url: base_url.trim_end_matches('/').to_owned(),
            nit: nit.to_owned(),
            organization_name: String::new(),
            user_email: String::new(),
            user_id: String::new(),
            roles: Vec::new(),
            portal_origin: String::new(),
            login_url: String::new(),
            username: String::new(),
            password: String::new(),
            ca_file: None,
            api_timeout: API_TIMEOUT,
            upload_timeout: UPLOAD_TIMEOUT,
            retry: RetryPolicy::default(),
            upload_retry: RetryPolicy::mutualser_upload(),
            signed_url_chunk: 50,
            filters_all_start: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap_or(NaiveDate::MIN),
            slow_call_every: Some(SLOW_CALL_EVERY),
        }
    }
}

/// The Mutual Ser client; implements [`RadApi`].
pub struct MutualSerClient {
    cfg: ClientConfig,
    http: Client,
    upload_http: Client,
    tokens: TokenCache,
    limiter: Arc<AdaptiveLimiter>,
    timings: Option<Arc<Timings>>,
    now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    new_id: Arc<dyn Fn() -> String + Send + Sync>,
}

impl MutualSerClient {
    /// Builds the client: its HTTP clients (the upload one never pins the CA),
    /// the cached login and the shared limiter.
    pub fn new(
        cfg: ClientConfig,
        limiter: Arc<AdaptiveLimiter>,
        timings: Option<Arc<Timings>>,
    ) -> anyhow::Result<MutualSerClient> {
        if cfg.base_url.trim().is_empty() || cfg.nit.trim().is_empty() {
            anyhow::bail!("mutualser: falta la URL base o el NIT");
        }
        let api_options = crate::http::ClientOptions::default()
            .with_user_agent(USER_AGENT)
            .with_timeout(cfg.api_timeout)
            .with_ca_file(cfg.ca_file.as_deref());
        let http = crate::http::build_client(&api_options)?;
        let upload_options = crate::http::ClientOptions::default()
            .with_user_agent(USER_AGENT)
            .with_timeout(cfg.upload_timeout);
        let upload_http = crate::http::build_client(&upload_options)?;
        let login = LoginConfig::from(&cfg);
        let login_http = login_client(&cfg)?;
        let tokens = TokenCache::new(TOKEN_SKEW, move || login.fetch(&login_http));
        Ok(MutualSerClient {
            cfg,
            http,
            upload_http,
            tokens,
            limiter,
            timings,
            now: Arc::new(Utc::now),
            new_id: Arc::new(|| uuid::Uuid::new_v4().to_string()),
        })
    }

    /// Replaces the clock (tests).
    #[must_use]
    pub fn with_clock(mut self, now: impl Fn() -> DateTime<Utc> + Send + Sync + 'static) -> Self {
        self.now = Arc::new(now);
        self
    }

    /// Replaces the id generator (tests).
    #[must_use]
    pub fn with_ids(mut self, new_id: impl Fn() -> String + Send + Sync + 'static) -> Self {
        self.new_id = Arc::new(new_id);
        self
    }

    fn new_id(&self) -> String {
        (self.new_id)()
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        let cfg = &self.cfg;
        let mut out = Vec::new();
        let mut set = |name: &'static str, value: &str| {
            if !value.is_empty() {
                out.push((name, value.to_owned()));
            }
        };
        set("Organizacion", &cfg.nit);
        set("Organizacionname", &cfg.organization_name);
        set("Email", &cfg.user_email);
        set("Usuario", &cfg.user_email);
        set("User-Id", &cfg.user_id);
        set("Roles", &cfg.roles.join(","));
        if !cfg.portal_origin.is_empty() {
            let origin = cfg.portal_origin.trim_end_matches('/');
            out.push(("Origin", origin.to_owned()));
            out.push(("Referer", format!("{origin}/")));
        }
        out.push(("Transaction-Id", self.new_id()));
        out.push(("User-Agent", MUTUALSER_USER_AGENT.to_owned()));
        out.push(("Content-Type", "application/json".to_owned()));
        out.push(("Accept", "application/json".to_owned()));
        out
    }

    /// Takes a limiter slot, recording the wait for the timings.
    fn acquire(&self, op: &str) -> crate::http::Permit<'_> {
        let waited = self.timings.as_ref().map(|t| t.now());
        let permit = self.limiter.acquire();
        if let (Some(tm), Some(t0)) = (&self.timings, waited) {
            tm.observe_limiter_wait(op, tm.since(t0));
            tm.observe_concurrency(self.limiter.current());
        }
        permit
    }

    /// `policy` with retries logged at Warn (host and path only, never the
    /// query string) and observed for the timings.
    fn observed_policy(
        &self,
        base: &RetryPolicy,
        op: &'static str,
        retried: &Arc<AtomicBool>,
    ) -> RetryPolicy {
        let mut policy = base.clone();
        let previous = base.on_retry.clone();
        let timings = self.timings.clone();
        let retried = retried.clone();
        policy.on_retry = Some(Arc::new(move |event| {
            retried.store(true, Ordering::Relaxed);
            if let Some(tm) = &timings {
                tm.observe_retry_wait(op, event.delay);
            }
            tracing::warn!(
                op,
                attempt = event.attempt,
                max_attempts = event.max_attempts,
                status = event.status,
                error = %event.error,
                retry_in_ms = event.delay.as_millis() as u64,
                host = %event.host,
                path = %event.path,
                "mutual ser request failed; retrying"
            );
            if let Some(previous) = &previous {
                previous(event);
            }
        }));
        policy
    }

    fn observe(&self, op: &str, started: Option<std::time::Instant>, failed: bool, retried: bool) {
        if let (Some(tm), Some(t0)) = (&self.timings, started) {
            tm.observe(op, tm.since(t0), failed, retried);
        }
    }

    /// Sends one authenticated API request and returns the answer body.
    fn call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> ApiResult<Vec<u8>> {
        let op = op_label(path);
        let mut url = reqwest::Url::parse(&format!("{}{path}", self.cfg.base_url))
            .map_err(|err| ApiError::Other(format!("mutualser: URL inválida: {err}")))?;
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        let payload = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|err| ApiError::Other(format!("mutualser: {err}")))?;
        let headers = self.headers();
        let permit = self.acquire(op);
        let started = self.timings.as_ref().map(|t| t.now());
        let retried = Arc::new(AtomicBool::new(false));
        let policy = self.observed_policy(&self.cfg.retry, op, &retried);
        let watch = SlowWatch::start(self.cfg.slow_call_every, op, &url);
        let result = send_with_auth(&self.tokens, &policy, None, |token| {
            let mut request = self.http.request(method.clone(), url.clone());
            for (name, value) in &headers {
                request = request.header(*name, value);
            }
            request = request.bearer_auth(token);
            match &payload {
                Some(bytes) => request.body(bytes.clone()),
                None => request,
            }
        });
        drop(watch);
        let outcome = match result {
            Ok(response) => read_answer(&method, path, response),
            Err(err) => Err((
                None,
                ApiError::Other(format!("mutualser: {method} {path}: {err}")),
            )),
        };
        let retried = retried.load(Ordering::Relaxed);
        match outcome {
            Ok((status, answer)) => {
                permit.finish(response_healthy(status));
                self.observe(op, started, false, retried);
                Ok(answer)
            }
            Err((status, err)) => {
                permit.finish(status.is_some_and(response_healthy));
                self.observe(op, started, true, retried);
                Err(err)
            }
        }
    }

    fn call_json(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> ApiResult<(Value, Vec<u8>)> {
        let raw = self.call(method.clone(), path, query, body)?;
        let value = serde_json::from_slice(&raw)
            .map_err(|err| ApiError::Other(format!("mutualser: {method} {path}: decode: {err}")))?;
        Ok((value, raw))
    }

    fn signed_urls(
        &self,
        names: &[String],
    ) -> ApiResult<std::collections::HashMap<String, String>> {
        let mut out = std::collections::HashMap::new();
        for chunk in names.chunks(self.cfg.signed_url_chunk.max(1)) {
            let (answer, _) = self.call_json(
                Method::GET,
                "/signedUrl/getUrlUploadFile",
                &[("fileNames", chunk.join(","))],
                None,
            )?;
            for name in chunk {
                let url = answer.get(name).map(flex).unwrap_or_default();
                if url.is_empty() {
                    return Err(ApiError::Other(format!(
                        "mutualser: no signed URL for {name}"
                    )));
                }
                out.insert(name.clone(), url);
            }
        }
        Ok(out)
    }

    fn put_all(
        &self,
        files: &[UploadFile],
        storage: &[String],
        urls: &std::collections::HashMap<String, String>,
    ) -> ApiResult<()> {
        let jobs: Vec<(&UploadFile, String)> = files
            .iter()
            .zip(storage)
            .map(|(file, name)| (file, urls.get(name).cloned().unwrap_or_default()))
            .collect();
        let failed = AtomicBool::new(false);
        let results = par::map(jobs, self.limiter.max().max(1), |(file, target)| {
            if failed.load(Ordering::Relaxed) {
                return Ok(());
            }
            let result = self.put(file, &target);
            if result.is_err() {
                failed.store(true, Ordering::Relaxed);
            }
            result
        });
        let errors: Vec<ApiError> = results.into_iter().filter_map(Result::err).collect();
        match errors.first() {
            None => Ok(()),
            Some(first) => Err(ApiError::Other(format!(
                "mutualser: {} of {} file uploads failed: {first}",
                errors.len(),
                files.len()
            ))),
        }
    }

    /// PUTs one file to its signed storage URL.
    fn put(&self, file: &UploadFile, target: &str) -> ApiResult<()> {
        let bytes = std::fs::read(&file.path)
            .map_err(|err| ApiError::Other(format!("upload {}: {err}", file.name)))?;
        let url = reqwest::Url::parse(target)
            .map_err(|_| ApiError::Other(format!("upload {}: URL firmada inválida", file.name)))?;
        let permit = self.acquire("put");
        let started = self.timings.as_ref().map(|t| t.now());
        let retried = Arc::new(AtomicBool::new(false));
        let policy = self.observed_policy(&self.cfg.upload_retry, "put", &retried);
        let watch = SlowWatch::start(self.cfg.slow_call_every, "put", &url);
        let result = send_with_retry(&policy, None, || {
            self.upload_http
                .put(url.clone())
                .header("Content-Type", "application/octet-stream")
                .body(bytes.clone())
        });
        drop(watch);
        let failure = match result {
            Err(err) => Some(format!("upload {}: {err}", file.name)),
            Ok(response) => {
                let code = response.status().as_u16();
                let mut sink = Vec::new();
                let _ = response.take(64 << 10).read_to_end(&mut sink);
                if code == 200 || code == 201 {
                    None
                } else {
                    Some(format!("upload {}: storage HTTP={code}", file.name))
                }
            }
        };
        permit.finish(failure.is_none());
        self.observe(
            "put",
            started,
            failure.is_some(),
            retried.load(Ordering::Relaxed),
        );
        failure.map_or(Ok(()), |message| Err(ApiError::Other(message)))
    }

    fn filter_xml_range(
        &self,
        invoice: &str,
        start: Option<&str>,
        end: Option<&str>,
    ) -> ApiResult<Option<XmlDocument>> {
        let body = json!({
            "documentNumber": invoice,
            "endDate": end,
            "invoiceNotificationStatusType": null,
            "numberId": self.cfg.nit,
            "startDate": start,
            "thirdType": "2",
            "uploadDate": null,
        });
        let (answer, _) = self.call_json(
            Method::POST,
            "/filter-xml/",
            &[("page", "0".to_owned()), ("req", "100".to_owned())],
            Some(&body),
        )?;
        let rows = answer.get("content").and_then(Value::as_array);
        Ok(rows
            .into_iter()
            .flatten()
            .find(|row| field(row, "billId").eq_ignore_ascii_case(invoice))
            .map(|row| XmlDocument {
                bill_id: field(row, "billId"),
                status: field(row, "status"),
                status_type: field(row, "statusType"),
                id: field(row, "id"),
                upload_date: field(row, "uploadDate"),
                validation_date: field(row, "validationDate"),
                error_codes: error_codes(row.get("errors"), row.get("mensajes")),
            }))
    }

    /// The `core/filtersAll` request of one invoice: the NIT, the date window
    /// (start .. tomorrow) and the first page of 100 rows.
    fn filters_all_body(&self, invoice: &str) -> Value {
        let now = (self.now)().with_timezone(&Local);
        let tomorrow = now + chrono::Duration::days(1);
        json!({
            "filtros": {
                "estado": "",
                "fecha_final": format!("{} 23:59:59", dmy(tomorrow)),
                "fecha_inicial": format!("{} 00:00:00", dmy_date(self.cfg.filters_all_start)),
                "numero_factura": invoice,
                "organizacion": self.cfg.nit,
                "remision": "",
                "tipo_documento": "",
            },
            "paginacion": {"pagina_actual": 1, "total": 100},
        })
    }
}

impl RadApi for MutualSerClient {
    /// `/upload`, `findLoad` (must be `EN_PROCESO`), signed URLs, the PUT of
    /// every file, then `/upload-files`.
    fn upload(&self, step: Step, files: &[UploadFile]) -> ApiResult<String> {
        let (type_id, ext) = step_type(step);
        if files.is_empty() {
            return Err(ApiError::Other("mutualser: empty upload".into()));
        }
        let id = self.new_id();
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        let storage: Vec<String> = files
            .iter()
            .map(|_| format!("{}.{ext}", self.new_id()))
            .collect();
        let body = json!({
            "cantidad": files.len(),
            "id_cargue": id,
            "id_tipo": type_id,
            "nombres": names,
            "organizacion": self.cfg.nit,
        });
        self.call(Method::POST, "/upload", &[], Some(&body))?;
        let load = self.find_load(&id, None)?;
        if load.status != "EN_PROCESO" {
            return Err(ApiError::Other(format!(
                "mutualser: upload {id} is not EN_PROCESO after creation (estado={:?})",
                load.status
            )));
        }
        let urls = self.signed_urls(&storage)?;
        self.put_all(files, &storage, &urls)?;
        let entries: Vec<Value> = files
            .iter()
            .zip(&storage)
            .map(|(file, code)| {
                json!({
                    "codigo": code,
                    "extension": ext,
                    "id_archivo": self.new_id(),
                    "id_cargue": id,
                    "id_tipo": type_id,
                    "mensajes": [],
                    "nombre": file.name,
                    "tamano": size_mb(file.size),
                })
            })
            .collect();
        self.call(
            Method::POST,
            "/upload-files",
            &[],
            Some(&Value::Array(entries)),
        )?;
        Ok(id)
    }

    fn find_load(&self, id_cargue: &str, since: Option<DateTime<Utc>>) -> ApiResult<Load> {
        let now = (self.now)();
        let since = since.filter(|s| *s <= now).unwrap_or(now);
        let tomorrow = now.with_timezone(&Local) + chrono::Duration::days(1);
        let body = json!({
            "fecha_final": format!("{} 23:59:59", dmy(tomorrow)),
            "fecha_inicial": format!("{} 00:00:00", dmy(since.with_timezone(&Local))),
            "id": id_cargue,
            "organizacion": self.cfg.nit,
        });
        let (answer, _) = self.call_json(Method::POST, "/findLoad", &[], Some(&body))?;
        let files = answer
            .get("archivos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(load_file)
            .collect();
        Ok(Load {
            status: field(&answer, "estado"),
            files,
            raw: answer.to_string(),
        })
    }

    /// Tries the legacy date ranges in order: no dates, today..tomorrow,
    /// 2025-01-01..tomorrow.
    fn filter_xml(&self, invoice: &str) -> ApiResult<Option<XmlDocument>> {
        let now = (self.now)().with_timezone(&Local);
        let iso = |d: DateTime<Local>| d.format("%Y-%m-%d").to_string();
        let (today, tomorrow) = (iso(now), iso(now + chrono::Duration::days(1)));
        let ranges: [(Option<&str>, Option<&str>); 3] = [
            (None, None),
            (Some(&today), Some(&tomorrow)),
            (Some("2025-01-01"), Some(&tomorrow)),
        ];
        for (start, end) in ranges {
            if let Some(doc) = self.filter_xml_range(invoice, start, end)? {
                return Ok(Some(doc));
            }
        }
        Ok(None)
    }

    fn filters_all(&self, invoice: &str) -> ApiResult<Option<FiltersAllItem>> {
        let body = self.filters_all_body(invoice);
        let (answer, _) = self.call_json(Method::POST, "/core/filtersAll", &[], Some(&body))?;
        let row = answer
            .get("contenido")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|row| field(row, "numero_factura").eq_ignore_ascii_case(invoice));
        Ok(row.map(filters_item))
    }

    fn file_details(&self, file_id: &str) -> ApiResult<Vec<FileDetail>> {
        let path = format!("/core/findDetailsByFile/{}", path_escape(file_id));
        let (answer, _) = self.call_json(Method::GET, &path, &[], None)?;
        Ok(answer
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| FileDetail {
                code: field(d, "codigo"),
                kind: field(d, "tipo"),
                field: field(d, "campo"),
                detail: field(d, "detalle"),
            })
            .collect())
    }
}

impl PortalApi for MutualSerClient {
    /// Sends the same `core/filtersAll` request as [`RadApi::filters_all`],
    /// parses every row and returns the one [`choose_record`] picks.
    fn portal_record(&self, invoice: &str) -> ApiResult<Option<PortalRecord>> {
        let body = self.filters_all_body(invoice);
        let (answer, _) = self.call_json(Method::POST, "/core/filtersAll", &[], Some(&body))?;
        let rows: Vec<PortalRecord> = answer
            .get("contenido")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(portal_row)
            .collect();
        Ok(choose_record(rows, invoice))
    }

    fn file_findings(&self, file_id: &str) -> ApiResult<Vec<PortalFinding>> {
        Ok(self
            .file_details(file_id)?
            .into_iter()
            .map(|d| PortalFinding {
                code: d.code,
                kind: d.kind,
                field: d.field,
                detail: d.detail,
            })
            .collect())
    }
}

/// A `core/filtersAll` row with every field the portal follow-up keeps.
fn portal_row(row: &Value) -> PortalRecord {
    let uploaded_at_text = field(row, "fecha_cargue");
    PortalRecord {
        invoice: field(row, "numero_factura"),
        status: field(row, "estado"),
        remission: field(row, "remision"),
        uploaded_at: parse_uploaded_at(&uploaded_at_text),
        uploaded_at_text,
        user: field(row, "usuario"),
        services: services_text(row.get("servicios")),
        modality: field(row, "modalidad"),
        file_name: field(row, "nombreArchivo"),
        file_id: field(row, "idArchivo"),
        upload_id: field(row, "idCargue"),
        event_id: field(row, "id_evento"),
        event_type: field(row, "tipoEvento"),
        error_type: field(row, "tipoError"),
        error_code: field(row, "codigoError"),
        returned: flex_bool(row.get("devuelto")),
        issue_date: field(row, "fecha_expedicion_factura"),
        receipt_name: field(row, "receipt_name"),
        support_document: row
            .get("documento_soporte")
            .filter(|s| s.is_object())
            .map(Value::to_string)
            .unwrap_or_default(),
        raw: row.to_string(),
    }
}

/// Short operation name used in logs and timings for an API path.
pub fn op_label(path: &str) -> &'static str {
    match path {
        "/upload" => "upload",
        "/upload-files" => "upload-files",
        "/findLoad" => "find-load",
        "/core/filtersAll" => "filters-all",
        p if p.starts_with("/signedUrl/") => "signed-urls",
        p if p.starts_with("/filter-xml") => "filter-xml",
        p if p.starts_with("/core/findDetailsByFile/") => "file-details",
        _ => "api",
    }
}

fn dmy(date: DateTime<Local>) -> String {
    format!("{:02}/{:02}/{}", date.day(), date.month(), date.year())
}

fn dmy_date(date: NaiveDate) -> String {
    format!("{:02}/{:02}/{}", date.day(), date.month(), date.year())
}

/// Percent-encodes a URL path segment.
fn path_escape(segment: &str) -> String {
    let mut out = String::new();
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Size in MB rounded to 2 decimals, as the portal reports it.
fn size_mb(bytes: u64) -> Value {
    let mb = (bytes as f64 / 1024.0 / 1024.0 * 100.0).round() / 100.0;
    if mb.fract() == 0.0 {
        return Value::from(mb as u64);
    }
    serde_json::Number::from_f64(mb).map_or(Value::Null, Value::Number)
}

/// A JSON string, number, bool or null as text (Go `flexString`).
fn flex(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `true`, `"true"` (any case) or `1` as true (Go `flexBool`).
fn flex_bool(value: Option<&Value>) -> bool {
    let text = value.map(flex).unwrap_or_default();
    text.eq_ignore_ascii_case("true") || text == "1"
}

fn field(value: &Value, name: &str) -> String {
    value.get(name).map(flex).unwrap_or_default()
}

/// `errors[].code|codigo` plus the codes of the messages of type ERROR.
fn error_codes(errors: Option<&Value>, messages: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    for e in errors.and_then(Value::as_array).into_iter().flatten() {
        let code = field(e, "code");
        let code = if code.is_empty() {
            field(e, "codigo")
        } else {
            code
        };
        if !code.is_empty() {
            out.push(code);
        }
    }
    for m in messages.and_then(Value::as_array).into_iter().flatten() {
        if field(m, "tipo").eq_ignore_ascii_case("ERROR") && !field(m, "codigo").is_empty() {
            out.push(field(m, "codigo"));
        }
    }
    out
}

fn load_file(entry: &Value) -> LoadFile {
    let messages = entry.get("mensajes");
    LoadFile {
        id: field(entry, "id"),
        name: field(entry, "nombre"),
        status: field(entry, "estado"),
        error_codes: error_codes(entry.get("errors"), messages),
        messages: messages
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|m| Message {
                kind: field(m, "tipo"),
                code: field(m, "codigo"),
                description: field(m, "descripcion"),
            })
            .collect(),
    }
}

/// The text of `servicios` (`""` when absent or null).
fn services_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn filters_item(row: &Value) -> FiltersAllItem {
    let services = services_text(row.get("servicios"));
    let support = row
        .get("documento_soporte")
        .filter(|s| s.is_object())
        .map(|s| SupportDocument {
            id: field(s, "id"),
            file_name: field(s, "nombre_archivo"),
            status: field(s, "estado"),
            raw: s.to_string(),
        });
    FiltersAllItem {
        invoice_number: field(row, "numero_factura"),
        status: field(row, "estado"),
        event_id: field(row, "id_evento"),
        file_id: field(row, "idArchivo"),
        id_cargue: field(row, "idCargue"),
        file_name: field(row, "nombreArchivo"),
        returned: flex_bool(row.get("devuelto")),
        is_support_document: flex_bool(row.get("isSupportDocument")),
        services_valid: !services.trim().is_empty(),
        services,
        support,
    }
}

/// Reads an answer: its status and body, or the error to report (with the
/// status when there was a response, for the limiter).
fn read_answer(
    method: &Method,
    path: &str,
    response: reqwest::blocking::Response,
) -> Result<(u16, Vec<u8>), (Option<u16>, ApiError)> {
    let status = response.status().as_u16();
    let mut raw = Vec::new();
    if let Err(err) = response.take(MAX_BODY).read_to_end(&mut raw) {
        return Err((
            Some(status),
            ApiError::Other(format!("mutualser: {method} {path}: read body: {err}")),
        ));
    }
    if !(200..300).contains(&status) {
        let body = String::from_utf8_lossy(&raw);
        let mut compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
        if compact.chars().count() > 400 {
            compact = compact.chars().take(400).collect::<String>() + "...";
        }
        return Err((
            Some(status),
            ApiError::Status {
                method: method.to_string(),
                path: path.to_owned(),
                code: status,
                body: compact,
            },
        ));
    }
    Ok((status, raw))
}

/// Logs a sign of life every `every` while an API call or PUT is in flight
/// (host and path only). Stops (and joins) when dropped; calls that finish
/// before the first tick log nothing.
struct SlowWatch {
    stop: Option<crossbeam_channel::Sender<()>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl SlowWatch {
    fn start(every: Option<Duration>, op: &'static str, url: &reqwest::Url) -> SlowWatch {
        let Some(every) = every.filter(|e| !e.is_zero()) else {
            return SlowWatch {
                stop: None,
                handle: None,
            };
        };
        let (stop, done) = crossbeam_channel::bounded::<()>(0);
        let (host, path) = (
            url.host_str().unwrap_or_default().to_owned(),
            url.path().to_owned(),
        );
        let started = std::time::Instant::now();
        let handle = std::thread::spawn(move || {
            while let Err(crossbeam_channel::RecvTimeoutError::Timeout) = done.recv_timeout(every) {
                tracing::info!(
                    op,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    host = %host,
                    path = %path,
                    "mutual ser is still answering slowly"
                );
            }
        });
        SlowWatch {
            stop: Some(stop),
            handle: Some(handle),
        }
    }
}

impl Drop for SlowWatch {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// The ZonaSer login (Go `tocumen`): `POST {username, password}` to the login
/// URL, no redirects (credentials must never follow one).
#[derive(Clone)]
struct LoginConfig {
    url: String,
    username: String,
    password: String,
}

impl From<&ClientConfig> for LoginConfig {
    fn from(cfg: &ClientConfig) -> Self {
        LoginConfig {
            url: cfg.login_url.clone(),
            username: cfg.username.clone(),
            password: cfg.password.clone(),
        }
    }
}

/// ZonaSer answers HTTP 500 to requests without a `User-Agent` (reqwest sends
/// none by default), so every client of this module sets one.
const USER_AGENT: &str = concat!("Tessia/", env!("CARGO_PKG_VERSION"));

/// A client that never follows redirects, with the ZonaSer CA pin.
fn login_client(cfg: &ClientConfig) -> anyhow::Result<Client> {
    use anyhow::Context;
    let mut builder = Client::builder()
        .user_agent(USER_AGENT)
        .timeout(cfg.api_timeout)
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(path) = cfg.ca_file.as_deref().filter(|p| !p.is_empty()) {
        let pem =
            std::fs::read(path).with_context(|| format!("no se pudo leer el archivo CA {path}"))?;
        let certs = reqwest::Certificate::from_pem_bundle(&pem).with_context(|| {
            format!("el archivo CA no contiene certificados PEM válidos: {path}")
        })?;
        builder = builder.tls_built_in_root_certs(false);
        for cert in certs {
            builder = builder.add_root_certificate(cert);
        }
    }
    builder
        .build()
        .context("no se pudo crear el cliente de login")
}

impl LoginConfig {
    fn fetch(&self, client: &Client) -> Result<FetchedToken, String> {
        if self.url.is_empty() || self.username.is_empty() || self.password.is_empty() {
            return Err("tocumen: login URL, username and password are required".into());
        }
        let response = client
            .post(&self.url)
            .header("Accept", "application/json")
            .json(&json!({"username": self.username, "password": self.password}))
            .send()
            .map_err(|err| format!("tocumen: login: {}", err.without_url()))?;
        let status = response.status();
        if status.is_redirection() {
            return Err(format!(
                "tocumen: login redirected (HTTP {})",
                status.as_u16()
            ));
        }
        if status.as_u16() != 200 {
            let mut raw = Vec::new();
            let _ = response.take(2048).read_to_end(&mut raw);
            return Err(format!(
                "tocumen: login failed (HTTP {}){}",
                status.as_u16(),
                self.body_hint(&raw)
            ));
        }
        let mut raw = Vec::new();
        let _ = response.take(1 << 20).read_to_end(&mut raw);
        let answer: Value = serde_json::from_slice(&raw)
            .map_err(|err| format!("tocumen: decode login response: {err}"))?;
        let token = field(&answer, "access_token");
        if token.is_empty() {
            return Err("tocumen: login response has no access_token".into());
        }
        let expires_in = answer
            .get("expires_in")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        Ok(FetchedToken {
            expires_at: Some(token_expiry(&token, expires_in)),
            token,
        })
    }
}

impl LoginConfig {
    /// A short single-line excerpt of an error body (` - cuerpo: ...`), with
    /// the configured password masked in case the server echoes it.
    fn body_hint(&self, raw: &[u8]) -> String {
        let mut text = String::from_utf8_lossy(raw).into_owned();
        if self.password.len() >= 4 {
            text = text.replace(self.password.as_str(), "[redacted]");
        }
        let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if one_line.is_empty() {
            return String::new();
        }
        let shown: String = one_line.chars().take(300).collect();
        let more = if one_line.chars().count() > 300 {
            "..."
        } else {
            ""
        };
        format!(" - cuerpo: {shown}{more}")
    }
}

fn token_expiry(token: &str, expires_in: i64) -> SystemTime {
    if expires_in > 0 {
        return SystemTime::now() + Duration::from_secs(expires_in as u64);
    }
    jwt_expiry(token).unwrap_or_else(|_| SystemTime::now() + DEFAULT_TOKEN_TTL)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::http::limiter::specs;
    use crate::testing::{MockResponse, MockServer, RecordedRequest};

    fn client_for(server: &MockServer) -> MutualSerClient {
        let mut cfg = ClientConfig::new(&format!("{}/rips-api/", server.url()), "900111222");
        cfg.login_url = format!("{}/login", server.url());
        cfg.username = "user".into();
        cfg.password = "secret".into();
        cfg.organization_name = "ORG".into();
        cfg.user_email = "ops@example.org".into();
        cfg.user_id = "u-1".into();
        cfg.roles = vec!["admin".into(), "ops".into()];
        cfg.portal_origin = "https://portal.example.org/".into();
        cfg.retry = RetryPolicy::instant(3);
        cfg.upload_retry = RetryPolicy::instant(10);
        cfg.slow_call_every = None;
        let limiter = Arc::new(AdaptiveLimiter::new(specs::MUTUALSER.options()));
        let counter = AtomicUsize::new(0);
        MutualSerClient::new(cfg, limiter, None)
            .expect("client")
            .with_ids(move || format!("id-{}", counter.fetch_add(1, Ordering::SeqCst) + 1))
    }

    fn login_ok(req: &RecordedRequest) -> Option<MockResponse> {
        (req.path == "/login")
            .then(|| MockResponse::json(200, r#"{"access_token":"tok-1","expires_in":3600}"#))
    }

    #[test]
    fn requests_carry_identity_headers_and_a_bearer_token() {
        let server = MockServer::start(|req| {
            login_ok(req).unwrap_or_else(|| {
                MockResponse::json(200, r#"{"estado":"TERMINADO","archivos":[]}"#)
            })
        });
        let client = client_for(&server);
        client.find_load("abc", None).expect("find load");
        let requests = server.requests();
        assert_eq!(requests[0].path, "/login");
        let login: Value = serde_json::from_slice(&requests[0].body).expect("login body");
        assert_eq!(login, json!({"username": "user", "password": "secret"}));
        let call = &requests[1];
        assert_eq!(
            (call.method.as_str(), call.path.as_str()),
            ("POST", "/rips-api/findLoad")
        );
        assert_eq!(call.header("authorization"), Some("Bearer tok-1"));
        assert_eq!(call.header("organizacion"), Some("900111222"));
        assert_eq!(call.header("organizacionname"), Some("ORG"));
        assert_eq!(call.header("email"), Some("ops@example.org"));
        assert_eq!(call.header("usuario"), Some("ops@example.org"));
        assert_eq!(call.header("user-id"), Some("u-1"));
        assert_eq!(call.header("roles"), Some("admin,ops"));
        assert_eq!(call.header("origin"), Some("https://portal.example.org"));
        assert_eq!(call.header("referer"), Some("https://portal.example.org/"));
        assert_eq!(call.header("user-agent"), Some(MUTUALSER_USER_AGENT));
        assert!(call.header("transaction-id").is_some());
    }

    #[test]
    fn a_401_refreshes_the_token_once_and_repeats_the_call() {
        let logins = Arc::new(AtomicUsize::new(0));
        let seen = logins.clone();
        let calls = Mutex::new(0);
        let server = MockServer::start(move |req| {
            if req.path == "/login" {
                let n = seen.fetch_add(1, Ordering::SeqCst) + 1;
                return MockResponse::json(
                    200,
                    &format!(r#"{{"access_token":"tok-{n}","expires_in":3600}}"#),
                );
            }
            let mut calls = calls.lock().expect("calls");
            *calls += 1;
            if *calls == 1 {
                MockResponse::text(401, "expired")
            } else {
                MockResponse::json(200, r#"{"estado":"TERMINADO"}"#)
            }
        });
        let load = client_for(&server)
            .find_load("abc", None)
            .expect("find load");
        assert_eq!(load.status, "TERMINADO");
        assert_eq!(logins.load(Ordering::SeqCst), 2);
        let auth: Vec<_> = server
            .requests()
            .iter()
            .filter(|r| r.path.ends_with("findLoad"))
            .map(|r| r.header("authorization").unwrap_or_default().to_owned())
            .collect();
        assert_eq!(auth, ["Bearer tok-1", "Bearer tok-2"]);
    }

    #[test]
    fn a_failed_login_is_reported_without_credentials() {
        let server = MockServer::start(|_| MockResponse::text(403, "no"));
        let err = client_for(&server)
            .find_load("abc", None)
            .expect_err("must fail");
        let text = err.to_string();
        assert!(text.contains("tocumen: login failed (HTTP 403)"), "{text}");
        assert!(!text.contains("secret"), "{text}");
    }

    #[test]
    fn non_2xx_answers_become_status_errors_with_a_short_body() {
        let server = MockServer::start(|req| {
            login_ok(req).unwrap_or_else(|| MockResponse::text(400, &"x ".repeat(500)))
        });
        let err = client_for(&server)
            .find_load("abc", None)
            .expect_err("must fail");
        match err {
            ApiError::Status { code, body, .. } => {
                assert_eq!(code, 400);
                assert!(body.len() <= 403, "{}", body.len());
                assert!(body.ends_with("..."));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn find_load_sends_the_date_window_and_parses_the_files() {
        let answer = r#"{"estado":"TERMINADO","archivos":[
            {"id":7,"nombre":"FEV1.txt","estado":"CARGADO","mensajes":[
                {"tipo":"ERROR","codigo":"RF-FE-013","descripcion":"CUV ya asociado"},
                {"tipo":"INFO","codigo":"I1","descripcion":"ok"}],
             "errors":[{"code":"E9"},{"codigo":"E8"}]}]}"#;
        let server = MockServer::start(move |req| {
            login_ok(req).unwrap_or_else(|| MockResponse::json(200, answer))
        });
        let client = client_for(&server);
        let now = DateTime::parse_from_rfc3339("2026-09-25T15:00:00Z")
            .expect("date")
            .with_timezone(&Utc);
        let client = client.with_clock(move || now);
        let since = DateTime::parse_from_rfc3339("2026-09-20T15:00:00Z")
            .expect("date")
            .with_timezone(&Utc);
        let load = client.find_load("up-1", Some(since)).expect("load");

        let body: Value = serde_json::from_slice(&server.requests()[1].body).expect("body");
        let local = |d: DateTime<Utc>| dmy(d.with_timezone(&Local));
        assert_eq!(body["id"], "up-1");
        assert_eq!(body["organizacion"], "900111222");
        assert_eq!(body["fecha_inicial"], format!("{} 00:00:00", local(since)));
        assert_eq!(
            body["fecha_final"],
            format!(
                "{} 23:59:59",
                dmy(now.with_timezone(&Local) + chrono::Duration::days(1))
            )
        );
        assert_eq!(load.status, "TERMINADO");
        let file = &load.files[0];
        assert_eq!(
            (file.id.as_str(), file.name.as_str(), file.status.as_str()),
            ("7", "FEV1.txt", "CARGADO")
        );
        assert_eq!(file.error_codes, ["E9", "E8", "RF-FE-013"]);
        assert_eq!(file.messages[0].description, "CUV ya asociado");
        assert!(
            load.raw
                .starts_with(r#"{"estado":"TERMINADO","archivos":[{"id":7"#),
            "{}",
            load.raw
        );
    }

    #[test]
    fn a_future_since_falls_back_to_today() {
        let server = MockServer::start(|req| {
            login_ok(req).unwrap_or_else(|| MockResponse::json(200, r#"{"estado":"EN_PROCESO"}"#))
        });
        let now = DateTime::parse_from_rfc3339("2026-09-25T15:00:00Z")
            .expect("date")
            .with_timezone(&Utc);
        let client = client_for(&server).with_clock(move || now);
        client
            .find_load("up", Some(now + chrono::Duration::days(3)))
            .expect("load");
        let body: Value = serde_json::from_slice(&server.requests()[1].body).expect("body");
        assert_eq!(
            body["fecha_inicial"],
            format!("{} 00:00:00", dmy(now.with_timezone(&Local)))
        );
    }

    #[test]
    fn filter_xml_walks_the_legacy_ranges_until_the_invoice_shows_up() {
        let calls = Mutex::new(0);
        let server = MockServer::start(move |req| {
            if let Some(login) = login_ok(req) {
                return login;
            }
            let mut calls = calls.lock().expect("calls");
            *calls += 1;
            if *calls < 3 {
                return MockResponse::json(
                    200,
                    r#"{"content":[{"billId":"OTHER","status":"ACTIVE"}]}"#,
                );
            }
            MockResponse::json(
                200,
                r#"{"content":[{"billId":"fev1","status":"ACTIVE","statusType":2,"id":"x1",
                    "uploadDate":"d1","validationDate":"d2","errors":[{"code":"C1"}]}]}"#,
            )
        });
        let now = DateTime::parse_from_rfc3339("2026-09-25T15:00:00Z")
            .expect("date")
            .with_timezone(&Utc);
        let doc = client_for(&server)
            .with_clock(move || now)
            .filter_xml("FEV1")
            .expect("filter")
            .expect("found");
        assert_eq!(
            (
                doc.bill_id.as_str(),
                doc.status.as_str(),
                doc.status_type.as_str()
            ),
            ("fev1", "ACTIVE", "2")
        );
        assert_eq!(doc.error_codes, ["C1"]);
        let requests: Vec<_> = server
            .requests()
            .into_iter()
            .filter(|r| r.path.starts_with("/rips-api/filter-xml"))
            .collect();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "/rips-api/filter-xml/?page=0&req=100");
        let ranges: Vec<(Value, Value)> = requests
            .iter()
            .map(|r| {
                let body: Value = serde_json::from_slice(&r.body).expect("body");
                (body["startDate"].clone(), body["endDate"].clone())
            })
            .collect();
        assert_eq!(ranges[0], (Value::Null, Value::Null));
        assert_eq!(ranges[2].0, json!("2025-01-01"));
        let body: Value = serde_json::from_slice(&requests[0].body).expect("body");
        assert_eq!(body["documentNumber"], "FEV1");
        assert_eq!(body["numberId"], "900111222");
        assert_eq!(body["thirdType"], "2");
    }

    #[test]
    fn filters_all_maps_the_row_of_the_invoice() {
        let answer = r#"{"contenido":[
            {"numero_factura":"OTHER","estado":"X"},
            {"numero_factura":"FEV1","estado":"VALIDADO","id_evento":5,"idArchivo":"f1","idCargue":"c1",
             "nombreArchivo":"FEV1.json","devuelto":"TRUE","isSupportDocument":true,
             "servicios":[{"a":1}],
             "documento_soporte":{"id":"s1","nombre_archivo":"CRC.zip","estado":"ACTIVE"}}]}"#;
        let server = MockServer::start(move |req| {
            login_ok(req).unwrap_or_else(|| MockResponse::json(200, answer))
        });
        let item = client_for(&server)
            .filters_all("FEV1")
            .expect("filters")
            .expect("found");
        assert_eq!(item.status, "VALIDADO");
        assert_eq!(
            (
                item.event_id.as_str(),
                item.file_id.as_str(),
                item.id_cargue.as_str()
            ),
            ("5", "f1", "c1")
        );
        assert!(item.returned && item.is_support_document && item.services_valid);
        assert_eq!(item.services, r#"[{"a":1}]"#);
        let support = item.support.expect("support");
        assert_eq!(
            (support.status.as_str(), support.file_name.as_str()),
            ("ACTIVE", "CRC.zip")
        );
        assert_eq!(
            support.raw,
            r#"{"id":"s1","nombre_archivo":"CRC.zip","estado":"ACTIVE"}"#
        );
        let body: Value = serde_json::from_slice(&server.requests()[1].body).expect("body");
        assert_eq!(body["filtros"]["numero_factura"], "FEV1");
        assert_eq!(body["filtros"]["organizacion"], "900111222");
        assert_eq!(body["filtros"]["fecha_inicial"], "01/01/2025 00:00:00");
        assert_eq!(
            body["paginacion"],
            json!({"total": 100, "pagina_actual": 1})
        );
    }

    #[test]
    fn filters_all_without_services_or_support() {
        let server = MockServer::start(|req| {
            login_ok(req).unwrap_or_else(|| {
                MockResponse::json(200, r#"{"contenido":[{"numero_factura":"FEV1","servicios":null,"documento_soporte":null}]}"#)
            })
        });
        let item = client_for(&server)
            .filters_all("FEV1")
            .expect("filters")
            .expect("found");
        assert!(!item.services_valid && item.support.is_none() && !item.returned);
        let none = MockServer::start(|req| {
            login_ok(req).unwrap_or_else(|| MockResponse::json(200, r#"{"contenido":[]}"#))
        });
        assert_eq!(
            client_for(&none).filters_all("FEV1").expect("filters"),
            None
        );
    }

    #[test]
    fn portal_record_picks_the_json_row_and_keeps_every_field() {
        let answer = r#"{"contenido":[
            {"numero_factura":"FEV1","estado":"OTRO","nombreArchivo":"CRC.zip","fecha_cargue":"29-09-2026 18:00:00"},
            {"numero_factura":"FEV1","estado":"VALIDADO_CON_ADVERTENCIA","remision":"R7","fecha_cargue":"29-09-2026 17:01:26",
             "usuario":"ops@example.org","servicios":"[1]","modalidad":"EVENTO","nombreArchivo":"FEV1.json",
             "idArchivo":"f1","idCargue":"c1","id_evento":9,"tipoEvento":"REGISTER_RIP","tipoError":"E","codigoError":"X1",
             "devuelto":false,"fecha_expedicion_factura":"2026-09-01","receipt_name":"rc",
             "documento_soporte":{"id":"s1"}}]}"#;
        let server = MockServer::start(move |req| {
            login_ok(req).unwrap_or_else(|| MockResponse::json(200, answer))
        });
        let record = client_for(&server)
            .portal_record("FEV1")
            .expect("record")
            .expect("found");
        assert_eq!(
            (record.status.as_str(), record.remission.as_str()),
            ("VALIDADO_CON_ADVERTENCIA", "R7")
        );
        assert_eq!(record.uploaded_at_text, "29-09-2026 17:01:26");
        assert_eq!(
            record.uploaded_at.map(|d| d.to_rfc3339()),
            Some("2026-09-29T22:01:26+00:00".to_owned())
        );
        assert_eq!(
            (
                record.file_id.as_str(),
                record.upload_id.as_str(),
                record.event_id.as_str()
            ),
            ("f1", "c1", "9")
        );
        assert_eq!(
            (
                record.event_type.as_str(),
                record.error_type.as_str(),
                record.error_code.as_str()
            ),
            ("REGISTER_RIP", "E", "X1")
        );
        assert_eq!(record.support_document, r#"{"id":"s1"}"#);
        assert!(record.raw.contains(r#""numero_factura":"FEV1""#));
        let none = MockServer::start(|req| {
            login_ok(req).unwrap_or_else(|| MockResponse::json(200, r#"{"contenido":[]}"#))
        });
        assert_eq!(
            client_for(&none).portal_record("FEV1").expect("record"),
            None
        );
    }

    #[test]
    fn file_details_escape_the_id_and_map_the_findings() {
        let server = MockServer::start(|req| {
            login_ok(req).unwrap_or_else(|| {
                MockResponse::json(
                    200,
                    r#"[{"codigo":"R1","tipo":"ERROR","campo":"edad","detalle":"mal"}]"#,
                )
            })
        });
        let details = client_for(&server).file_details("a b/1").expect("details");
        assert_eq!(
            server.requests()[1].path,
            "/rips-api/core/findDetailsByFile/a%20b%2F1"
        );
        assert_eq!(
            details,
            [FileDetail {
                code: "R1".into(),
                kind: "ERROR".into(),
                field: "edad".into(),
                detail: "mal".into()
            }]
        );
    }

    /// A mock of the whole upload exchange; storage PUTs go to the same server.
    fn upload_server(put_failures: usize, load_status: &'static str) -> MockServer {
        let failures = Mutex::new(put_failures);
        MockServer::start(move |req| {
            if let Some(login) = login_ok(req) {
                return login;
            }
            let path = req.path.split('?').next().unwrap_or_default().to_owned();
            match path.as_str() {
                "/rips-api/upload" | "/rips-api/upload-files" => MockResponse::json(200, "{}"),
                "/rips-api/findLoad" => {
                    MockResponse::json(200, &format!(r#"{{"estado":"{load_status}"}}"#))
                }
                "/rips-api/signedUrl/getUrlUploadFile" => {
                    let host = req.header("host").unwrap_or_default().to_owned();
                    let names = req
                        .path
                        .split("fileNames=")
                        .nth(1)
                        .unwrap_or_default()
                        .replace("%2C", ",");
                    let map: serde_json::Map<String, Value> = names
                        .split(',')
                        .map(|n| {
                            (
                                n.to_owned(),
                                json!(format!("http://{host}/storage/{n}?sig=SECRET")),
                            )
                        })
                        .collect();
                    MockResponse::json(200, &Value::Object(map).to_string())
                }
                p if p.starts_with("/storage/") => {
                    let mut left = failures.lock().expect("failures");
                    if *left > 0 {
                        *left -= 1;
                        MockResponse::text(503, "busy")
                    } else {
                        MockResponse::text(200, "")
                    }
                }
                _ => MockResponse::text(404, ""),
            }
        })
    }

    fn files_on_disk(dir: &std::path::Path, names: &[(&str, &str)]) -> Vec<UploadFile> {
        names
            .iter()
            .map(|(name, content)| {
                let path = dir.join(name);
                std::fs::write(&path, content).expect("write");
                UploadFile {
                    invoice: "FEV1".into(),
                    rel: (*name).to_owned(),
                    path,
                    name: (*name).to_owned(),
                    size: content.len() as u64,
                }
            })
            .collect()
    }

    #[test]
    fn upload_registers_puts_and_confirms_in_order() {
        let server = upload_server(0, "EN_PROCESO");
        let dir = tempfile::tempdir().expect("dir");
        let files = files_on_disk(dir.path(), &[("FEV1.txt", "{}"), ("FEV2.txt", "{\"a\":1}")]);
        let id = client_for(&server)
            .upload(Step::Cuv, &files)
            .expect("upload");
        assert_eq!(id, "id-1");

        let requests = server.requests();
        let paths: Vec<String> = requests
            .iter()
            .map(|r| {
                format!(
                    "{} {}",
                    r.method,
                    r.path.split('?').next().unwrap_or_default()
                )
            })
            .collect();
        assert_eq!(paths[1], "POST /rips-api/upload");
        assert_eq!(paths[2], "POST /rips-api/findLoad");
        assert_eq!(paths[3], "GET /rips-api/signedUrl/getUrlUploadFile");
        let puts: Vec<_> = paths
            .iter()
            .filter(|p| p.starts_with("PUT /storage/"))
            .collect();
        assert_eq!(puts.len(), 2);
        assert_eq!(
            paths.last().map(String::as_str),
            Some("POST /rips-api/upload-files")
        );

        let register: Value = serde_json::from_slice(&requests[1].body).expect("body");
        assert_eq!(
            register,
            json!({"cantidad": 2, "id_cargue": "id-1", "id_tipo": "5072926906777600",
                   "nombres": ["FEV1.txt", "FEV2.txt"], "organizacion": "900111222"})
        );
        let confirm: Value =
            serde_json::from_slice(&requests.last().expect("last").body).expect("body");
        let entries = confirm.as_array().expect("array");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["nombre"], "FEV1.txt");
        assert_eq!(entries[0]["id_cargue"], "id-1");
        assert_eq!(entries[0]["extension"], "txt");
        assert_eq!(entries[0]["id_tipo"], "5072926906777600");
        assert_eq!(entries[0]["tamano"], 0.0);
        assert!(
            entries[0]["codigo"]
                .as_str()
                .expect("codigo")
                .ends_with(".txt")
        );
        assert_eq!(entries[0]["mensajes"], json!([]));
        let put = requests.iter().find(|r| r.method == "PUT").expect("put");
        assert_eq!(put.header("content-type"), Some("application/octet-stream"));
        assert_eq!(
            put.header("authorization"),
            None,
            "signed URLs carry no bearer token"
        );
    }

    #[test]
    fn puts_are_retried_and_the_storage_names_are_unique_uuid_like_ids() {
        let server = upload_server(2, "EN_PROCESO");
        let dir = tempfile::tempdir().expect("dir");
        let files = files_on_disk(dir.path(), &[("FEV1.json", "{}")]);
        client_for(&server)
            .upload(Step::Rips, &files)
            .expect("upload");
        let puts = server
            .requests()
            .iter()
            .filter(|r| r.method == "PUT")
            .count();
        assert_eq!(puts, 3, "two 503 answers, then success");
    }

    #[test]
    fn upload_fails_when_the_upload_is_not_en_proceso_after_creation() {
        let server = upload_server(0, "TERMINADO");
        let dir = tempfile::tempdir().expect("dir");
        let files = files_on_disk(dir.path(), &[("FEV1.txt", "{}")]);
        let err = client_for(&server)
            .upload(Step::Cuv, &files)
            .expect_err("must fail");
        assert!(
            err.to_string().contains("is not EN_PROCESO after creation"),
            "{err}"
        );
        assert!(server.requests().iter().all(|r| r.method != "PUT"));
    }

    #[test]
    fn a_put_that_keeps_failing_fails_the_upload_and_never_registers_the_files() {
        let server = upload_server(100, "EN_PROCESO");
        let dir = tempfile::tempdir().expect("dir");
        let files = files_on_disk(dir.path(), &[("FEV1.txt", "{}")]);
        let err = client_for(&server)
            .upload(Step::Cuv, &files)
            .expect_err("must fail");
        let text = err.to_string();
        assert!(text.contains("1 of 1 file uploads failed"), "{text}");
        assert!(text.contains("storage HTTP=503"), "{text}");
        assert!(
            !text.contains("SECRET"),
            "signed query must never be reported: {text}"
        );
        assert!(
            server
                .requests()
                .iter()
                .all(|r| !r.path.ends_with("upload-files"))
        );
    }

    #[test]
    fn upload_sizes_are_megabytes_with_two_decimals() {
        assert_eq!(size_mb(0), json!(0));
        assert_eq!(size_mb(1024 * 1024), json!(1));
        assert_eq!(size_mb(1_572_864), json!(1.5));
        assert_eq!(size_mb(7 * 1024 * 1024 / 100), json!(0.07));
    }

    #[test]
    fn step_types_and_operation_labels() {
        assert_eq!(
            step_type(Step::InvoiceZip),
            ("1da64d49-f5f7-4d70-8a7c-640db4816521", "zip")
        );
        assert_eq!(step_type(Step::Cuv).1, "txt");
        assert_eq!(step_type(Step::Rips), ("5643548095414272", "json"));
        assert_eq!(step_type(Step::Crc), ("5655362556395520", "zip"));
        assert_eq!(op_label("/upload"), "upload");
        assert_eq!(op_label("/signedUrl/getUrlUploadFile"), "signed-urls");
        assert_eq!(op_label("/filter-xml/"), "filter-xml");
        assert_eq!(op_label("/core/findDetailsByFile/9"), "file-details");
    }

    #[test]
    fn missing_base_url_or_nit_is_an_error() {
        let limiter = Arc::new(AdaptiveLimiter::new(specs::MUTUALSER.options()));
        assert!(MutualSerClient::new(ClientConfig::new("", "1"), limiter.clone(), None).is_err());
        assert!(MutualSerClient::new(ClientConfig::new("http://x", " "), limiter, None).is_err());
    }
}
