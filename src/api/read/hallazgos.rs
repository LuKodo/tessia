//! Owned by one Wave 4 writer; see `odd/reference/ui-api-spec.md`.

use crate::api::{
    ApiState,
    errors::{ApiError, ApiResult},
    request::Request,
};

pub fn list(_state: &ApiState, _req: &Request) -> ApiResult {
    Err(ApiError::new(501, "not implemented"))
}

pub fn update(_state: &ApiState, _req: &Request) -> ApiResult {
    Err(ApiError::new(501, "not implemented"))
}
