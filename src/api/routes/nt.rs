//! `GET /api/nota-tecnica`: read-only status of the Nota Técnica kept in Redis.
//!
//! Go reported the shared workbook (`exists`, `file_name`, `size_bytes`,
//! `modified_at`) and accepted uploads. The collection is now maintained
//! outside this program (S6), so this answers `{"exists", "file_name",
//! "version", "filas"}` from the current version and its metadata hash, and
//! there is no upload route.

use serde::Serialize;

use crate::api::{ApiError, ApiResult, ApiState, request::Request, request::Response};

/// Message of the 503 when Redis cannot be read (the driver error is not echoed).
pub const NT_UNAVAILABLE: &str = "no se pudo consultar la nota técnica";

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct NotaTecnicaStatus {
    /// A current version exists.
    pub exists: bool,
    /// The version, which names the loaded collection (empty when none).
    pub file_name: String,
    pub version: String,
    /// Row count from the version's `filas` metadata (empty when unknown).
    pub filas: String,
}

pub fn get(state: &ApiState, _req: &Request) -> ApiResult {
    let nt = &state.ctx.nt;
    let unavailable = |err: crate::nt::NtError| {
        tracing::warn!(error = %err, "nota tecnica: no se pudo leer Redis");
        ApiError::unavailable(NT_UNAVAILABLE)
    };
    let status = match nt.current_version().map_err(unavailable)? {
        None => NotaTecnicaStatus {
            exists: false,
            file_name: String::new(),
            version: String::new(),
            filas: String::new(),
        },
        Some(version) => {
            let meta = nt.meta(&version).map_err(unavailable)?;
            NotaTecnicaStatus {
                exists: true,
                file_name: version.clone(),
                filas: meta.get("filas").cloned().unwrap_or_default(),
                version,
            }
        }
    };
    Ok(Response::json(&status))
}
