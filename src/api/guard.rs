//! Same-origin guard for the state-changing routes (Go: `serve_origin.go`).
//!
//! The API has no authentication and a custom protocol can be reached by any
//! page loaded in the webview, so a mutating request is trusted only when it
//! comes from the app itself or carries no browser origin at all.

use std::collections::HashSet;
use std::sync::RwLock;

use super::{errors::ApiError, request::Request};

/// Message of the 403 answered to a request from another origin.
pub const CROSS_ORIGIN_MESSAGE: &str = "solicitud de otro origen no permitida";

/// Origins of the Wails app: Windows serves it from `http://wails.localhost`,
/// macOS and Linux from `wails://wails`.
pub const DEFAULT_APP_ORIGINS: [&str; 2] = ["http://wails.localhost", "wails://wails"];

/// The set of app origins plus the rules that decide a request's origin.
#[derive(Debug)]
pub struct OriginGuard {
    allowed: RwLock<HashSet<String>>,
}

impl Default for OriginGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl OriginGuard {
    /// A guard that trusts [`DEFAULT_APP_ORIGINS`].
    pub fn new() -> Self {
        let guard = Self {
            allowed: RwLock::new(HashSet::new()),
        };
        for origin in DEFAULT_APP_ORIGINS {
            guard.allow(origin);
        }
        guard
    }

    /// Trusts one more origin (the shell registers the origin its custom
    /// protocol uses). Blank values are ignored.
    pub fn allow(&self, origin: &str) {
        let origin = normalize(origin);
        if origin.is_empty() {
            return;
        }
        if let Ok(mut set) = self.allowed.write() {
            set.insert(origin);
        }
    }

    /// True when `origin` is a registered app origin.
    pub fn is_allowed(&self, origin: &str) -> bool {
        self.allowed
            .read()
            .map(|set| set.contains(&normalize(origin)))
            .unwrap_or(false)
    }

    /// Rejects a cross-site request. Order, as in Go:
    /// 1. an `Origin` that is a registered app origin passes;
    /// 2. `Sec-Fetch-Site: cross-site` is rejected;
    /// 3. no `Origin` passes (scripts and tooling, never a page);
    /// 4. otherwise the `Origin` must be http(s) with the same host as the
    ///    request's `Host` header; without a `Host` it cannot match.
    pub fn check(&self, req: &Request) -> Result<(), ApiError> {
        let origin = req.header("origin").map(str::trim).unwrap_or("");
        if !origin.is_empty() && self.is_allowed(origin) {
            return Ok(());
        }
        if req
            .header("sec-fetch-site")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("cross-site"))
        {
            return Err(cross_origin());
        }
        if origin.is_empty() {
            return Ok(());
        }
        if same_host(origin, req.header("host")) {
            Ok(())
        } else {
            Err(cross_origin())
        }
    }
}

fn cross_origin() -> ApiError {
    ApiError::forbidden(CROSS_ORIGIN_MESSAGE)
}

fn normalize(origin: &str) -> String {
    origin.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// Go's `u.Scheme in (http, https) && u.Host != "" && EqualFold(u.Host, r.Host)`.
fn same_host(origin: &str, host: Option<&str>) -> bool {
    let Some(host) = host.map(str::trim).filter(|h| !h.is_empty()) else {
        return false;
    };
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        return false;
    }
    let Some(origin_host) = url.host_str() else {
        return false;
    };
    let origin_host = match url.port() {
        Some(port) => format!("{origin_host}:{port}"),
        None => origin_host.to_owned(),
    };
    origin_host.eq_ignore_ascii_case(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post() -> Request {
        Request::new("POST", "/api/x")
    }

    #[test]
    fn no_origin_headers_pass() {
        assert!(OriginGuard::new().check(&post()).is_ok());
    }

    #[test]
    fn app_origins_pass_even_when_labelled_cross_site() {
        let guard = OriginGuard::new();
        for origin in ["http://wails.localhost", "wails://wails", "WAILS://wails/"] {
            let req = post()
                .with_header("Origin", origin)
                .with_header("Sec-Fetch-Site", "cross-site");
            assert!(guard.check(&req).is_ok(), "{origin}");
        }
    }

    #[test]
    fn cross_site_and_foreign_origins_are_rejected() {
        let guard = OriginGuard::new();
        let cross = post().with_header("Sec-Fetch-Site", "cross-site");
        let foreign = post().with_header("Origin", "https://evil.example");
        let null = post().with_header("Origin", "null");
        for req in [cross, foreign, null] {
            let err = guard.check(&req).expect_err("rejected");
            assert_eq!(
                (err.status, err.message.as_str()),
                (403, CROSS_ORIGIN_MESSAGE)
            );
        }
    }

    #[test]
    fn same_host_origin_passes_only_with_a_matching_host_header() {
        let guard = OriginGuard::new();
        let ok = post()
            .with_header("Origin", "http://app.test:8080")
            .with_header("Host", "APP.test:8080");
        assert!(guard.check(&ok).is_ok());
        let other_port = post()
            .with_header("Origin", "http://app.test:8080")
            .with_header("Host", "app.test:9090");
        assert!(guard.check(&other_port).is_err());
        let no_host = post().with_header("Origin", "http://app.test:8080");
        assert!(guard.check(&no_host).is_err());
        let wrong_scheme = post()
            .with_header("Origin", "ftp://app.test")
            .with_header("Host", "app.test");
        assert!(guard.check(&wrong_scheme).is_err());
    }

    #[test]
    fn a_registered_origin_is_trusted_and_blank_ones_are_ignored() {
        let guard = OriginGuard::new();
        let req = post().with_header("Origin", "tessia://localhost");
        assert!(guard.check(&req).is_err());
        guard.allow("tessia://localhost/");
        guard.allow("  ");
        assert!(guard.check(&req).is_ok());
        assert!(!guard.is_allowed(""));
    }
}
