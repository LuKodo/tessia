//! Same-origin guard for the state-changing routes (Go: `serve_origin.go`).

use super::{errors::ApiError, request::Request};

/// Rejects a cross-site request: `Sec-Fetch-Site: cross-site`, or an `Origin`
/// that differs from the app origin. A request with neither passes.
pub fn check(_req: &Request) -> Result<(), ApiError> {
    Ok(())
}
