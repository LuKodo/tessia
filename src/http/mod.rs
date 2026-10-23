//! HTTP building blocks shared by every client (port of Go `internal/platform/httpx`).
//!
//! * [`client`]: `reqwest` blocking client builder (timeouts, pinned CA file,
//!   cookie jar, gzip);
//! * [`retry`]: [`RetryPolicy`] + [`send_with_retry`] (backoff with jitter,
//!   `Retry-After`, idempotency rules, optional limiter);
//! * [`limiter`]: [`AdaptiveLimiter`], the AIMD concurrency limiter, and the
//!   per-system numbers Go used ([`limiter::specs`]);
//! * [`token`]: [`TokenCache`] (single-flight cached bearer tokens),
//!   [`send_with_auth`] (re-login once on 401) and [`jwt_expiry`].
//!
//! ```no_run
//! use tessia::http::{self, ClientOptions, RetryPolicy, send_with_retry};
//!
//! # fn demo() -> anyhow::Result<()> {
//! let client = http::build_client(&ClientOptions::default())?;
//! let response = send_with_retry(&RetryPolicy::default(), None, || client.get("https://example.org/x"))?;
//! println!("{}", response.status());
//! # Ok(()) }
//! ```

pub mod client;
pub mod limiter;
pub mod retry;
pub mod token;

pub use client::{ClientOptions, build_client, safe_url};
pub use limiter::{
    AdaptiveLimiter, LimiterOptions, LimiterSpec, Limiters, Permit, response_healthy,
};
pub use retry::{HttpError, RetryEvent, RetryPolicy, send_with_retry};
pub use token::{TokenCache, jwt_expiry, send_with_auth};

/// Browser User-Agent Medicar requires (the portal answers 403 to non-browser
/// agents such as Postman).
pub const MEDICAR_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

/// Browser User-Agent Mutual Ser (ZonaSer / portal) expects.
pub const MUTUALSER_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";
