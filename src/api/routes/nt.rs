//! `GET /api/nota-tecnica`: read-only status of the Nota Técnica kept in Redis.
//!
//! Go reported the shared workbook (`exists`, `file_name`, `size_bytes`,
//! `modified_at`) and accepted uploads. The collection is now maintained
//! outside this program (S6), so this answers `{"exists", "file_name",
//! "version", "filas"}` from the current version and its metadata hash, and
//! there is no upload route.

use serde::Serialize;

use crate::api::{ApiResult, ApiState, request::Request, request::Response};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct NotaTecnicaStatus {
    /// A current version exists.
    pub exists: bool,
    /// First 12 characters of the version (empty when none).
    pub file_name: String,
    pub version: String,
    /// Row count from the version's `filas` metadata (empty when unknown).
    pub filas: String,
}

pub fn get(state: &ApiState, _req: &Request) -> ApiResult {
    let nt = &state.ctx.nt;
    // The card polls this every minute: an unreachable Redis is "nothing published", never an error.
    let none = NotaTecnicaStatus {
        exists: false,
        file_name: String::new(),
        version: String::new(),
        filas: String::new(),
    };
    let unavailable = |err: crate::nt::NtError| {
        tracing::warn!(error = %err, "nota tecnica: no se pudo leer Redis");
    };
    let version = match nt.current_version() {
        Ok(version) => version,
        Err(err) => {
            unavailable(err);
            return Ok(Response::json(&none));
        }
    };
    let status = match version {
        None => none,
        Some(version) => {
            let meta = match nt.meta(&version) {
                Ok(meta) => meta,
                Err(err) => {
                    unavailable(err);
                    return Ok(Response::json(&none));
                }
            };
            NotaTecnicaStatus {
                exists: true,
                file_name: version.chars().take(12).collect(),
                filas: meta.get("filas").cloned().unwrap_or_default(),
                version,
            }
        }
    };
    Ok(Response::json(&status))
}
