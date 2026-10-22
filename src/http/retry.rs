//! Retries with exponential backoff and jitter (port of Go `httpx.Do`).
//!
//! Retries happen on transport errors, 408, 425, 429 and 5xx. Because a
//! retried POST/PATCH may duplicate a fiscal or accounting document,
//! non-idempotent requests are retried only on 425/429 (the server did not
//! process them) unless [`RetryPolicy::retry_non_idempotent`] is set or the
//! request carries an `Idempotency-Key` header.

use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use reqwest::blocking::{RequestBuilder, Response};
use reqwest::header::RETRY_AFTER;
use reqwest::{Method, StatusCode};

use super::client::safe_url;
use super::limiter::{AdaptiveLimiter, response_healthy};

/// Describes one failed attempt that is about to be retried. It never carries
/// the query string, userinfo, headers or bodies, so it is safe to log.
#[derive(Debug, Clone)]
pub struct RetryEvent {
    pub method: String,
    pub host: String,
    pub path: String,
    /// 1-based attempt that just failed; `max_attempts` includes it.
    pub attempt: u32,
    pub max_attempts: u32,
    /// Backoff slept before the next attempt.
    pub delay: Duration,
    /// Status that triggered the retry (0 for transport errors).
    pub status: u16,
    /// Sanitized description of a transport error ("" for status retries).
    pub error: String,
}

/// Retry settings. `Default` is the Go default: 3 attempts, 250 ms doubling up
/// to 10 s, +-20% jitter.
#[derive(Clone)]
pub struct RetryPolicy {
    /// Attempts including the first.
    pub max_attempts: u32,
    /// First backoff; doubles per attempt.
    pub base_delay: Duration,
    /// Caps one backoff, `Retry-After` included.
    pub max_delay: Duration,
    /// +- fraction applied to each delay (0 disables).
    pub jitter: f64,
    /// Also retry POST/PATCH on transport errors, 408 and 5xx.
    pub retry_non_idempotent: bool,
    /// Called right before each backoff sleep.
    pub on_retry: Option<Arc<dyn Fn(&RetryEvent) + Send + Sync>>,
    /// Replaces `thread::sleep` (tests).
    pub sleeper: Option<Arc<dyn Fn(Duration) + Send + Sync>>,
}

impl std::fmt::Debug for RetryPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetryPolicy")
            .field("max_attempts", &self.max_attempts)
            .field("base_delay", &self.base_delay)
            .field("max_delay", &self.max_delay)
            .field("jitter", &self.jitter)
            .field("retry_non_idempotent", &self.retry_non_idempotent)
            .finish_non_exhaustive()
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(10),
            jitter: 0.2,
            retry_non_idempotent: false,
            on_retry: None,
            sleeper: None,
        }
    }
}

impl RetryPolicy {
    /// Mutual Ser uploads: 10 attempts, 1 s doubling up to 2 minutes.
    pub fn mutualser_upload() -> Self {
        Self {
            max_attempts: 10,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(120),
            ..Self::default()
        }
    }

    /// A policy that never sleeps (tests).
    pub fn instant(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            sleeper: Some(Arc::new(|_| {})),
            ..Self::default()
        }
    }

    /// Backoff after the given failed attempt (1-based), jitter included.
    pub fn backoff(&self, attempt: u32) -> Duration {
        let exponent = attempt.saturating_sub(1).min(20);
        let delay = self
            .base_delay
            .saturating_mul(1u32 << exponent)
            .min(self.max_delay);
        let jitter = self.jitter.clamp(0.0, 1.0);
        let delay = if jitter > 0.0 {
            let factor = 1.0 + jitter * (2.0 * rand::rng().random::<f64>() - 1.0);
            delay.mul_f64(factor)
        } else {
            delay
        };
        delay.min(self.max_delay)
    }

    fn sleep(&self, delay: Duration) {
        match &self.sleeper {
            Some(sleeper) => sleeper(delay),
            None => std::thread::sleep(delay),
        }
    }
}

/// Failure of [`send_with_retry`]: a transport error (no response at all) or a
/// request that could not be built. The message never contains the query
/// string or credentials.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("{method} {url}: se rindió tras {attempts} intentos: {source}")]
    Transport {
        method: String,
        url: String,
        attempts: u32,
        source: reqwest::Error,
    },
    #[error("no se pudo construir la solicitud HTTP: {0}")]
    Build(reqwest::Error),
}

fn is_idempotent(method: &Method, has_idempotency_key: bool) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE | Method::PUT | Method::DELETE
    ) || has_idempotency_key
}

fn retryable_status(status: StatusCode, idempotent_ok: bool) -> bool {
    match status.as_u16() {
        425 | 429 => true,
        408 | 500..=599 => idempotent_ok,
        _ => false,
    }
}

/// Parses `Retry-After` (seconds or HTTP date).
fn retry_after(response: &Response) -> Duration {
    let Some(value) = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
    else {
        return Duration::ZERO;
    };
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Duration::from_secs(seconds);
    }
    httpdate_to_delay(value)
}

fn httpdate_to_delay(value: &str) -> Duration {
    chrono::DateTime::parse_from_rfc2822(value.trim())
        .ok()
        .and_then(|when| {
            (when.with_timezone(&chrono::Utc) - chrono::Utc::now())
                .to_std()
                .ok()
        })
        .unwrap_or(Duration::ZERO)
}

/// Sends the request built by `build`, retrying per `policy`.
///
/// `build` is called once per attempt, so the body is naturally re-sendable
/// (build it from owned bytes or re-open the file inside). When attempts are
/// exhausted on a retryable *status* the last response is returned as `Ok`, so
/// callers can inspect it; only transport errors become `Err`. With a
/// `limiter`, every attempt takes a slot while the request is in flight (until
/// the headers arrive) and gives it back before the backoff sleep. To hold a
/// slot while the body is read, use [`AdaptiveLimiter::exchange`] instead and
/// pass `None` here.
pub fn send_with_retry(
    policy: &RetryPolicy,
    limiter: Option<&AdaptiveLimiter>,
    mut build: impl FnMut() -> RequestBuilder,
) -> Result<Response, HttpError> {
    let max_attempts = policy.max_attempts.max(1);
    let mut attempt = 1;
    loop {
        let (client, request) = build().build_split();
        let request = request.map_err(HttpError::Build)?;
        let idempotent_ok = policy.retry_non_idempotent
            || is_idempotent(
                request.method(),
                request.headers().contains_key("idempotency-key")
                    || request.headers().contains_key("x-idempotency-key"),
            );
        let (method, url) = (request.method().clone(), request.url().clone());
        let permit = limiter.map(AdaptiveLimiter::acquire);
        let outcome = client.execute(request);

        let (delay, status, error) = match outcome {
            Err(err) => {
                if let Some(permit) = permit {
                    permit.finish(false);
                }
                if !idempotent_ok || attempt >= max_attempts {
                    return Err(HttpError::Transport {
                        method: method.to_string(),
                        url: safe_url(&url),
                        attempts: attempt,
                        source: err.without_url(),
                    });
                }
                (policy.backoff(attempt), 0, describe_error(err))
            }
            Ok(response) => {
                let code = response.status();
                if let Some(permit) = permit {
                    permit.finish(response_healthy(code.as_u16()));
                }
                if !(retryable_status(code, idempotent_ok) && attempt < max_attempts) {
                    return Ok(response);
                }
                let delay = policy
                    .backoff(attempt)
                    .max(retry_after(&response))
                    .min(policy.max_delay);
                (delay, code.as_u16(), String::new())
            }
        };
        if let Some(on_retry) = &policy.on_retry {
            on_retry(&RetryEvent {
                method: method.to_string(),
                host: url.host_str().unwrap_or_default().to_owned(),
                path: url.path().to_owned(),
                attempt,
                max_attempts,
                delay,
                status,
                error,
            });
        }
        policy.sleep(delay);
        attempt += 1;
    }
}

/// Sanitized transport error text: the cause without the URL, bounded.
fn describe_error(err: reqwest::Error) -> String {
    const MAX: usize = 200;
    let kind = if err.is_timeout() {
        "timeout"
    } else if err.is_connect() {
        "connect"
    } else {
        "request"
    };
    let mut text = format!("{kind}: {}", err.without_url());
    if text.len() > MAX {
        text.truncate(MAX);
        text.push_str("...");
    }
    text
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::testing::{MockResponse, MockServer};

    fn client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("client")
    }

    fn recording_policy(attempts: u32) -> (RetryPolicy, Arc<Mutex<Vec<Duration>>>) {
        let sleeps = Arc::new(Mutex::new(Vec::new()));
        let log = sleeps.clone();
        let policy = RetryPolicy {
            max_attempts: attempts,
            jitter: 0.0,
            sleeper: Some(Arc::new(move |d| log.lock().expect("log").push(d))),
            ..RetryPolicy::default()
        };
        (policy, sleeps)
    }

    #[test]
    fn get_is_retried_on_5xx_until_it_succeeds() {
        let server = MockServer::sequence(vec![
            MockResponse::text(503, "no"),
            MockResponse::text(500, "no"),
            MockResponse::text(200, "ok"),
        ]);
        let (policy, sleeps) = recording_policy(3);
        let c = client();
        let response = send_with_retry(&policy, None, || c.get(server.url())).expect("response");
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(server.requests().len(), 3);
        assert_eq!(
            *sleeps.lock().expect("log"),
            [Duration::from_millis(250), Duration::from_millis(500)]
        );
    }

    #[test]
    fn exhausted_status_retries_return_the_last_response() {
        let server = MockServer::sequence(vec![MockResponse::text(502, "bad gateway")]);
        let c = client();
        let response = send_with_retry(&RetryPolicy::instant(3), None, || c.get(server.url()))
            .expect("response");
        assert_eq!(response.status().as_u16(), 502);
        assert_eq!(server.requests().len(), 3);
    }

    #[test]
    fn post_is_not_retried_on_5xx_unless_allowed_but_is_on_429() {
        let c = client();
        let server = MockServer::sequence(vec![
            MockResponse::text(503, "no"),
            MockResponse::text(200, "ok"),
        ]);
        let response = send_with_retry(&RetryPolicy::instant(3), None, || {
            c.post(server.url()).body("x")
        })
        .expect("response");
        assert_eq!(response.status().as_u16(), 503);
        assert_eq!(server.requests().len(), 1);

        let throttled = MockServer::sequence(vec![
            MockResponse::text(429, "slow"),
            MockResponse::text(200, "ok"),
        ]);
        let response = send_with_retry(&RetryPolicy::instant(3), None, || {
            c.post(throttled.url()).body("x")
        })
        .expect("response");
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(throttled.requests().len(), 2);
        assert!(
            throttled.requests().iter().all(|r| r.body_text() == "x"),
            "the body is re-sent"
        );

        let allowed = MockServer::sequence(vec![
            MockResponse::text(503, "no"),
            MockResponse::text(200, "ok"),
        ]);
        let policy = RetryPolicy {
            retry_non_idempotent: true,
            ..RetryPolicy::instant(3)
        };
        let response =
            send_with_retry(&policy, None, || c.post(allowed.url()).body("x")).expect("response");
        assert_eq!(response.status().as_u16(), 200);

        let keyed = MockServer::sequence(vec![
            MockResponse::text(503, "no"),
            MockResponse::text(200, "ok"),
        ]);
        let response = send_with_retry(&RetryPolicy::instant(3), None, || {
            c.post(keyed.url()).header("Idempotency-Key", "k").body("x")
        })
        .expect("response");
        assert_eq!(response.status().as_u16(), 200);
    }

    #[test]
    fn client_errors_are_returned_without_retrying() {
        let server = MockServer::sequence(vec![MockResponse::text(404, "missing")]);
        let c = client();
        let response = send_with_retry(&RetryPolicy::instant(5), None, || c.get(server.url()))
            .expect("response");
        assert_eq!(response.status().as_u16(), 404);
        assert_eq!(server.requests().len(), 1);
    }

    #[test]
    fn retry_after_is_honored_but_capped() {
        let server = MockServer::sequence(vec![
            MockResponse::text(429, "slow").with_header("Retry-After", "3"),
            MockResponse::text(429, "slow").with_header("Retry-After", "3600"),
            MockResponse::text(200, "ok"),
        ]);
        let (policy, sleeps) = recording_policy(3);
        let c = client();
        send_with_retry(&policy, None, || c.get(server.url())).expect("response");
        assert_eq!(
            *sleeps.lock().expect("log"),
            [Duration::from_secs(3), Duration::from_secs(10)]
        );
    }

    #[test]
    fn transport_errors_are_retried_then_reported_without_the_query() {
        // Nothing listens on this port.
        let url = "http://127.0.0.1:9/never?token=SECRET";
        let c = client();
        let events = Arc::new(Mutex::new(Vec::new()));
        let log = events.clone();
        let policy = RetryPolicy {
            on_retry: Some(Arc::new(move |e| log.lock().expect("log").push(e.clone()))),
            ..RetryPolicy::instant(2)
        };
        let err = send_with_retry(&policy, None, || c.get(url)).expect_err("transport error");
        let text = err.to_string();
        assert!(
            text.contains("tras 2 intentos") && !text.contains("SECRET"),
            "{text}"
        );
        let events = events.lock().expect("log");
        assert_eq!(events.len(), 1);
        assert_eq!(
            (events[0].attempt, events[0].status, events[0].path.as_str()),
            (1, 0, "/never")
        );
        assert!(!events[0].error.contains("SECRET"));
    }

    #[test]
    fn backoff_doubles_and_is_capped_with_bounded_jitter() {
        let policy = RetryPolicy {
            jitter: 0.0,
            ..RetryPolicy::default()
        };
        assert_eq!(policy.backoff(1), Duration::from_millis(250));
        assert_eq!(policy.backoff(3), Duration::from_secs(1));
        assert_eq!(policy.backoff(10), Duration::from_secs(10));
        let jittered = RetryPolicy::default();
        for _ in 0..50 {
            let d = jittered.backoff(1);
            assert!(
                d >= Duration::from_millis(200) && d <= Duration::from_millis(300),
                "{d:?}"
            );
        }
    }

    #[test]
    fn the_limiter_sees_health_of_every_attempt() {
        use crate::http::limiter::LimiterOptions;
        let limiter = AdaptiveLimiter::new(LimiterOptions {
            name: "t".into(),
            min: 1,
            initial: 4,
            max: 4,
            window: 20,
            pause_min: Duration::ZERO,
            pause_max: Duration::ZERO,
        });
        let server = MockServer::sequence(vec![MockResponse::text(200, "ok")]);
        let c = client();
        for _ in 0..3 {
            send_with_retry(&RetryPolicy::instant(1), Some(&limiter), || {
                c.get(server.url())
            })
            .expect("response");
        }
        assert_eq!(limiter.active(), 0);
    }
}
