//! The Mutual Ser filing API as the engine sees it (Go `radicacion.API`). The
//! only implementation is [`super::client::MutualSerClient`]; the trait exists
//! so the engine can be tested without HTTP.

use chrono::{DateTime, Utc};

use super::model::{FileDetail, FiltersAllItem, Load, UploadFile, XmlDocument};
use crate::domain::radicacion::Step;

/// Failure of a Mutual Ser call.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Non-2xx answer (the body is truncated to 400 characters).
    #[error("mutualser: {method} {path}: HTTP {code}: {body}")]
    Status {
        method: String,
        path: String,
        code: u16,
        body: String,
    },
    /// Transport failure, login failure or unreadable answer.
    #[error("{0}")]
    Other(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Mutual Ser (ZonaSer benavente) filing API.
pub trait RadApi: Send + Sync {
    /// Registers `files` under one new upload id, uploads their bytes and
    /// returns the id. It does not wait for validation.
    fn upload(&self, step: Step, files: &[UploadFile]) -> ApiResult<String>;
    /// Queries an upload; `since` is the upload date (`None` = today), which
    /// bounds the date window Mutual Ser searches.
    fn find_load(&self, id_cargue: &str, since: Option<DateTime<Utc>>) -> ApiResult<Load>;
    /// The `filter-xml` row of `invoice`, trying the legacy date ranges.
    fn filter_xml(&self, invoice: &str) -> ApiResult<Option<XmlDocument>>;
    fn filters_all(&self, invoice: &str) -> ApiResult<Option<FiltersAllItem>>;
    fn file_details(&self, file_id: &str) -> ApiResult<Vec<FileDetail>>;
}
