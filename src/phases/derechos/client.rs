//! HTTP client of Mutual Ser's rights validator: a Keycloak password-grant
//! token and the FHIR `validateRights` endpoint.
//!
//! Port of Go `internal/adapters/derechosapi`:
//!
//! * 2xx: parse the bundle (an empty bundle is "not found");
//! * 400-410 (except 401/403): retry with the alternate document type
//!   (`TI -> CC`, `RC -> TI -> CC`); when none is left, "not found";
//! * 401: the rejected token is renewed (single flight, shared by every
//!   worker) and the request resent once with the current token;
//! * 401/403 after that, and any failure to obtain a token:
//!   [`ValidatorError::Authentication`], which stops the whole run;
//! * transport errors, 5xx and 429 after the retries (4 attempts), invalid
//!   JSON: [`ValidatorError::Technical`].
//!
//! One client per process is shared by every call ([`shared_client`]): one
//! login, one single-flight renewal and one adaptive limiter for the phase.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::json;

use super::rights::{AffiliateQuery, Rights, alternate_doc_type, parse_response};
use crate::config::Settings;
use crate::http::limiter::specs;
use crate::http::token::{AuthError, FetchedToken};
use crate::http::{
    AdaptiveLimiter, ClientOptions, RetryPolicy, TokenCache, build_client, jwt_expiry,
    send_with_auth, send_with_retry,
};

/// Whole-request timeout of the validator (Go: 30 s).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Tokens are refreshed this long before their expiry.
const TOKEN_SKEW: Duration = Duration::from_secs(30);
/// Attempts of one request (Java: 3 retries on timeouts).
const MAX_ATTEMPTS: u32 = 4;
const MAX_BODY: u64 = 8 << 20;
/// Settings required to query the validator.
pub const REQUIRED_KEYS: [&str; 6] = [
    "derechos.token_url",
    "derechos.validate_url",
    "derechos.client_id",
    "derechos.client_secret",
    "derechos.username",
    "derechos.password",
];

/// Why a validation call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidatorError {
    /// A failure of this affiliate's query (network, 5xx, invalid JSON).
    #[error("derechos: validator unavailable: {0}")]
    Technical(String),
    /// The token or credentials are rejected: retrying other affiliates
    /// cannot help.
    #[error("derechos: validator authentication failed: {0}")]
    Authentication(String),
}

/// One validator answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup {
    /// `None`: the affiliate was not found.
    pub rights: Option<Rights>,
    /// The document type that answered (after `TI -> CC` / `RC -> TI`).
    pub queried_doc_type: String,
}

/// The `derechos.*` configuration of a client. Two equal configurations share
/// one process-wide client.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientConfig {
    pub token_url: String,
    pub validate_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub username: String,
    pub password: String,
    pub ca_file: Option<String>,
}

impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig").finish_non_exhaustive()
    }
}

impl ClientConfig {
    /// Reads the `derechos.*` keys; the error names the missing keys and
    /// their environment variables, never values.
    pub fn from_settings(settings: &Settings) -> Result<Self> {
        settings.require(&REQUIRED_KEYS)?;
        let get = |key: &str| settings.get(key).unwrap_or_default().to_owned();
        Ok(Self {
            token_url: get("derechos.token_url"),
            validate_url: get("derechos.validate_url"),
            client_id: get("derechos.client_id"),
            client_secret: get("derechos.client_secret"),
            username: get("derechos.username"),
            password: get("derechos.password"),
            ca_file: settings
                .get("derechos.ca_file")
                .filter(|p| !p.is_empty())
                .map(str::to_owned),
        })
    }
}

/// The validator client: HTTP client, cached token and adaptive limiter.
pub struct ValidatorClient {
    config: ClientConfig,
    http: Client,
    tokens: TokenCache,
    limiter: AdaptiveLimiter,
    retry: RetryPolicy,
}

impl std::fmt::Debug for ValidatorClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatorClient").finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    expires_in: i64,
}

impl ValidatorClient {
    /// A client whose limiter ceiling is `max_concurrency` (at least 4) and
    /// whose retries are the production ones.
    pub fn new(config: ClientConfig, max_concurrency: usize) -> Result<Self> {
        Self::with_retry(config, max_concurrency, production_retry())
    }

    /// Like [`new`](Self::new) with another retry policy (tests use an instant one).
    pub fn with_retry(
        config: ClientConfig,
        max_concurrency: usize,
        retry: RetryPolicy,
    ) -> Result<Self> {
        let options = ClientOptions::default()
            .with_ca_file(config.ca_file.as_deref())
            .with_timeout(REQUEST_TIMEOUT);
        let http =
            build_client(&options).context("no se pudo crear el cliente HTTP de derechos")?;
        let fetch = {
            let (http, config, retry) = (http.clone(), config.clone(), retry.clone());
            move || fetch_token(&http, &config, &retry)
        };
        Ok(Self {
            tokens: TokenCache::new(TOKEN_SKEW, fetch),
            limiter: AdaptiveLimiter::new(specs::derechos(max_concurrency).options()),
            config,
            http,
            retry,
        })
    }

    /// The adaptive limiter shared by every call of this client.
    pub fn limiter(&self) -> &AdaptiveLimiter {
        &self.limiter
    }

    /// Validates one affiliate, following the document-type fallbacks.
    pub fn validate(&self, affiliate: &AffiliateQuery) -> Result<Lookup, ValidatorError> {
        let mut doc_type = affiliate.doc_type.clone();
        for _ in 0..3 {
            let (status, body) = self.query(&doc_type, &affiliate.doc_number)?;
            match status {
                200..=299 => {
                    let rights = parse_response(&body).map_err(ValidatorError::Technical)?;
                    return Ok(Lookup {
                        rights,
                        queried_doc_type: doc_type,
                    });
                }
                401 | 403 => {
                    return Err(ValidatorError::Authentication(format!(
                        "HTTP {status} (credentials rejected after renewing the token)"
                    )));
                }
                400..=410 => match alternate_doc_type(&doc_type) {
                    Some(alternate) => doc_type = alternate.to_owned(),
                    None => return Ok(not_found(doc_type)),
                },
                _ => return Err(ValidatorError::Technical(format!("HTTP {status}"))),
            }
        }
        Ok(not_found(doc_type))
    }

    fn query(&self, doc_type: &str, doc_number: &str) -> Result<(u16, Vec<u8>), ValidatorError> {
        let payload = request_body(doc_type, doc_number);
        let response = send_with_auth(&self.tokens, &self.retry, Some(&self.limiter), |token| {
            self.http
                .post(&self.config.validate_url)
                .header(CONTENT_TYPE, "application/json")
                .header(ACCEPT, "*/*")
                .bearer_auth(token)
                .body(payload.clone())
        })
        .map_err(|err| match err {
            AuthError::Token(err) => ValidatorError::Authentication(err.to_string()),
            AuthError::Http(err) => ValidatorError::Technical(err.to_string()),
        })?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        response
            .take(MAX_BODY)
            .read_to_end(&mut body)
            .map_err(|err| ValidatorError::Technical(format!("read response: {err}")))?;
        Ok((status, body))
    }
}

fn not_found(queried_doc_type: String) -> Lookup {
    Lookup {
        rights: None,
        queried_doc_type,
    }
}

fn production_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: MAX_ATTEMPTS,
        retry_non_idempotent: true,
        ..RetryPolicy::default()
    }
}

/// FHIR `Parameters` with `documentType` and `documentId`.
fn request_body(doc_type: &str, doc_number: &str) -> Vec<u8> {
    json!({
        "resourceType": "Parameters",
        "id": "CorrelationId",
        "parameter": [
            {"name": "documentType", "valueString": doc_type},
            {"name": "documentId", "valueString": doc_number},
        ],
    })
    .to_string()
    .into_bytes()
}

fn fetch_token(
    http: &Client,
    config: &ClientConfig,
    retry: &RetryPolicy,
) -> Result<FetchedToken, String> {
    let response = send_with_retry(retry, None, || {
        http.post(&config.token_url).form(&[
            ("grant_type", "password"),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("username", config.username.as_str()),
            ("password", config.password.as_str()),
        ])
    })
    .map_err(|err| format!("derechosapi: token: {err}"))?;
    if response.status().as_u16() != 200 {
        return Err(format!(
            "derechosapi: token: HTTP {}",
            response.status().as_u16()
        ));
    }
    let mut raw = Vec::new();
    response
        .take(1 << 20)
        .read_to_end(&mut raw)
        .map_err(|err| format!("derechosapi: token: {err}"))?;
    let answer: TokenAnswer = serde_json::from_slice(&raw)
        .ok()
        .filter(|a: &TokenAnswer| !a.access_token.is_empty())
        .ok_or("derechosapi: token: response without access_token")?;
    let expires_at = if answer.expires_in > 0 {
        SystemTime::now() + Duration::from_secs(answer.expires_in.unsigned_abs())
    } else {
        jwt_expiry(&answer.access_token)
            .unwrap_or_else(|_| SystemTime::now() + Duration::from_secs(300))
    };
    Ok(FetchedToken {
        token: answer.access_token,
        expires_at: Some(expires_at),
    })
}

type SharedClients = Mutex<Vec<(ClientConfig, Arc<ValidatorClient>)>>;
static SHARED: SharedClients = Mutex::new(Vec::new());

/// The process-wide validator client for `config`, built on first use.
/// `proceso run` calls the phase once per invoice, so a client per call would
/// mean one login, one token cache and one limiter per invoice. `max` is the
/// limiter ceiling requested by the call that builds it; a different
/// configuration (which a real process never has) gets its own client.
pub fn shared_client(
    config: &ClientConfig,
    max_concurrency: usize,
) -> Result<Arc<ValidatorClient>> {
    let mut clients = SHARED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((_, client)) = clients.iter().find(|(current, _)| current == config) {
        return Ok(client.clone());
    }
    let client = Arc::new(ValidatorClient::new(
        config.clone(),
        max_concurrency.max(4),
    )?);
    clients.push((config.clone(), client.clone()));
    Ok(client)
}
