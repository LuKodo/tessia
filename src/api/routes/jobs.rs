//! Owned by one Wave 4 writer; see `odd/reference/ui-api-spec.md`.

use crate::api::{
    ApiState,
    errors::{ApiError, ApiResult},
    request::Request,
};

pub fn run(_state: &ApiState, _req: &Request) -> ApiResult {
    Err(ApiError::new(501, "not implemented"))
}

pub fn download(_state: &ApiState, _req: &Request) -> ApiResult {
    Err(ApiError::new(501, "not implemented"))
}

pub fn reintentar(_state: &ApiState, _req: &Request) -> ApiResult {
    Err(ApiError::new(501, "not implemented"))
}

pub fn current(_state: &ApiState, _req: &Request) -> ApiResult {
    Err(ApiError::new(501, "not implemented"))
}

pub fn history(_state: &ApiState, _req: &Request) -> ApiResult {
    Err(ApiError::new(501, "not implemented"))
}

pub fn cancel(_state: &ApiState, _req: &Request) -> ApiResult {
    Err(ApiError::new(501, "not implemented"))
}
