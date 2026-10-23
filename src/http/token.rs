//! Cached bearer tokens with single-flight refresh, 401 re-login and JWT
//! expiry decoding (port of Go `httpx/token.go`).

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use base64::Engine;
use reqwest::StatusCode;
use reqwest::blocking::{RequestBuilder, Response};

use super::limiter::AdaptiveLimiter;
use super::retry::{HttpError, RetryPolicy, send_with_retry};

/// Why a token could not be obtained.
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// The login failed or returned no token (as opposed to the request that
    /// needed it failing).
    #[error("no se pudo obtener el token: {0}")]
    Fetch(String),
}

/// What the fetch function returns.
#[derive(Debug, Clone)]
pub struct FetchedToken {
    pub token: String,
    /// `None`: no known expiry (cached until invalidated).
    pub expires_at: Option<SystemTime>,
}

type FetchFn = dyn Fn() -> Result<FetchedToken, String> + Send + Sync;

#[derive(Default)]
struct Cached {
    token: String,
    expires_at: Option<SystemTime>,
    valid: bool,
}

/// A token cache. Concurrent callers share one in-flight fetch (the fetch runs
/// while the cache lock is held, so the waiters then read the fresh token).
pub struct TokenCache {
    fetch: Box<FetchFn>,
    skew: Duration,
    cached: Mutex<Cached>,
}

impl TokenCache {
    /// `fetch` obtains a new token; tokens are refreshed `skew` before their
    /// expiry.
    pub fn new(
        skew: Duration,
        fetch: impl Fn() -> Result<FetchedToken, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            fetch: Box::new(fetch),
            skew,
            cached: Mutex::new(Cached::default()),
        }
    }

    /// The cached token, or a freshly fetched one.
    pub fn token(&self) -> Result<String, TokenError> {
        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let fresh = cached.valid
            && cached
                .expires_at
                .is_none_or(|exp| SystemTime::now() + self.skew < exp);
        if fresh {
            return Ok(cached.token.clone());
        }
        let fetched = (self.fetch)().map_err(TokenError::Fetch)?;
        if fetched.token.is_empty() {
            return Err(TokenError::Fetch("token vacío".into()));
        }
        *cached = Cached {
            token: fetched.token.clone(),
            expires_at: fetched.expires_at,
            valid: true,
        };
        Ok(fetched.token)
    }

    /// Drops the cached token so the next call fetches again.
    pub fn invalidate(&self) {
        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *cached = Cached::default();
    }

    /// Drops the cached token only if it is still `rejected`: a late 401 for a
    /// token another caller already replaced must not discard the renewed one.
    pub fn invalidate_token(&self, rejected: &str) {
        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cached.valid && cached.token == rejected {
            *cached = Cached::default();
        }
    }
}

/// Failure of [`send_with_auth`].
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error(transparent)]
    Token(#[from] TokenError),
    #[error(transparent)]
    Http(#[from] HttpError),
}

/// Sends the request built by `build(token)` with a token from `cache`. On
/// 401 it invalidates the rejected token and resends exactly once with the
/// renewed one.
pub fn send_with_auth(
    cache: &TokenCache,
    policy: &RetryPolicy,
    limiter: Option<&AdaptiveLimiter>,
    mut build: impl FnMut(&str) -> RequestBuilder,
) -> Result<Response, AuthError> {
    let token = cache.token()?;
    let response = send_with_retry(policy, limiter, || build(&token))?;
    if response.status() != StatusCode::UNAUTHORIZED {
        return Ok(response);
    }
    cache.invalidate_token(&token);
    let renewed = cache.token()?;
    Ok(send_with_retry(policy, limiter, || build(&renewed))?)
}

/// Decodes the `exp` claim of a JWT without verifying its signature. Only for
/// cache timing of tokens obtained over TLS (e.g. SISPRO).
pub fn jwt_expiry(token: &str) -> Result<SystemTime, String> {
    let token = token.trim().trim_start_matches("Bearer ").trim();
    let payload = token.split('.').nth(1).ok_or("no es un JWT")?;
    let payload = payload.trim_end_matches('=');
    let engine = &base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let bytes = engine
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(payload))
        .map_err(|err| format!("payload JWT: {err}"))?;
    let claims: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|err| format!("payload JWT: {err}"))?;
    let exp = claims
        .get("exp")
        .and_then(serde_json::Value::as_f64)
        .ok_or("el JWT no tiene exp")?;
    if !exp.is_finite() || exp < 0.0 {
        return Err("exp inválido".into());
    }
    Ok(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(exp))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use super::*;
    use crate::testing::{MockResponse, MockServer};

    fn counting_cache(ttl: Option<Duration>) -> (Arc<TokenCache>, Arc<AtomicUsize>) {
        let fetches = Arc::new(AtomicUsize::new(0));
        let counter = fetches.clone();
        let cache = TokenCache::new(Duration::from_secs(1), move || {
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(FetchedToken {
                token: format!("t{n}"),
                expires_at: ttl.map(|d| SystemTime::now() + d),
            })
        });
        (Arc::new(cache), fetches)
    }

    #[test]
    fn tokens_are_cached_until_near_expiry() {
        let (cache, fetches) = counting_cache(Some(Duration::from_secs(60)));
        assert_eq!(cache.token().expect("t"), "t1");
        assert_eq!(cache.token().expect("t"), "t1");
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        let (expiring, fetches) = counting_cache(Some(Duration::from_millis(500)));
        assert_eq!(expiring.token().expect("t"), "t1");
        assert_eq!(
            expiring.token().expect("t"),
            "t2",
            "inside the skew window it refreshes"
        );
        assert_eq!(fetches.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn concurrent_callers_share_one_fetch() {
        let (cache, fetches) = counting_cache(None);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let cache = cache.clone();
                thread::spawn(move || cache.token().expect("token"))
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().expect("worker"), "t1");
        }
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn invalidate_token_only_drops_the_rejected_token() {
        let (cache, _) = counting_cache(None);
        assert_eq!(cache.token().expect("t"), "t1");
        cache.invalidate_token("stale");
        assert_eq!(cache.token().expect("t"), "t1");
        cache.invalidate_token("t1");
        assert_eq!(cache.token().expect("t"), "t2");
    }

    #[test]
    fn fetch_failures_and_empty_tokens_are_errors() {
        let failing = TokenCache::new(Duration::ZERO, || Err("login rechazado".into()));
        assert!(
            failing
                .token()
                .expect_err("fails")
                .to_string()
                .contains("login rechazado")
        );
        let empty = TokenCache::new(Duration::ZERO, || {
            Ok(FetchedToken {
                token: String::new(),
                expires_at: None,
            })
        });
        assert!(empty.token().is_err());
    }

    #[test]
    fn a_401_triggers_one_relogin_and_one_resend() {
        let server = MockServer::start(|req| match req.header("authorization") {
            Some("Bearer t2") => MockResponse::text(200, "ok"),
            _ => MockResponse::text(401, "no"),
        });
        let (cache, fetches) = counting_cache(None);
        let client = reqwest::blocking::Client::new();
        let response = send_with_auth(&cache, &RetryPolicy::instant(1), None, |token| {
            client.get(server.url()).bearer_auth(token)
        })
        .expect("response");
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(fetches.load(Ordering::SeqCst), 2);
        assert_eq!(server.requests().len(), 2);
    }

    #[test]
    fn a_second_401_is_returned_to_the_caller() {
        let server = MockServer::sequence(vec![MockResponse::text(401, "no")]);
        let (cache, _) = counting_cache(None);
        let client = reqwest::blocking::Client::new();
        let response = send_with_auth(&cache, &RetryPolicy::instant(1), None, |t| {
            client.get(server.url()).bearer_auth(t)
        })
        .expect("response");
        assert_eq!(response.status().as_u16(), 401);
        assert_eq!(server.requests().len(), 2);
    }

    #[test]
    fn jwt_expiry_reads_the_exp_claim() {
        let engine = &base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload = engine.encode(br#"{"exp":1800000000,"sub":"x"}"#);
        let token = format!("Bearer aaa.{payload}.sig");
        let exp = jwt_expiry(&token).expect("exp");
        assert_eq!(
            exp.duration_since(SystemTime::UNIX_EPOCH)
                .expect("after epoch")
                .as_secs(),
            1_800_000_000
        );
        assert!(jwt_expiry("nodots").is_err());
        let no_exp = format!("a.{}.c", engine.encode(br#"{"sub":"x"}"#));
        assert!(jwt_expiry(&no_exp).is_err());
    }
}
