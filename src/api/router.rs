//! Route table and dispatch. Fallbacks answer 404 `ruta no encontrada` and 405
//! `método no permitido` (with `Allow`), as Go's router does.

use std::panic::{AssertUnwindSafe, catch_unwind};

use super::{
    ApiError, ApiResult, ApiState, read,
    request::{Request, Response},
    routes,
};

type Handler = fn(&ApiState, &Request) -> ApiResult;

/// `(method, pattern, handler)`; `{x}` captures one path segment.
const ROUTES: &[(&str, &str, Handler)] = &[
    ("GET", "/api/health", routes::health::get),
    ("GET", "/api/nota-tecnica", routes::nt::get),
    ("GET", "/api/cargues", read::cargues::list),
    ("GET", "/api/cargues/{id}/informe", read::informe::get),
    (
        "POST",
        "/api/cargues/{id}/reintentar",
        routes::jobs::reintentar,
    ),
    ("GET", "/api/reportes/radicadas", read::radicadas::get),
    ("GET", "/api/seguimiento/consulta", read::consulta::get),
    ("GET", "/api/resumen", read::resumen::get),
    ("GET", "/api/facturas/{f}/seguimiento", read::tracking::get),
    ("GET", "/api/facturas/{f}/portal", read::portal::get),
    (
        "POST",
        "/api/facturas/{f}/portal/actualizar",
        read::portal::refresh,
    ),
    ("GET", "/api/hallazgos", read::hallazgos::list),
    ("PUT", "/api/hallazgos/{c}", read::hallazgos::update),
    ("GET", "/api/parameters", routes::parametros::get),
    ("PUT", "/api/parameters", routes::parametros::put),
    ("POST", "/api/convenio/upload", routes::upload::post),
    ("POST", "/api/download", routes::jobs::download),
    ("POST", "/api/run", routes::jobs::run),
    ("GET", "/api/jobs/current", routes::jobs::current),
    ("GET", "/api/jobs/history", routes::jobs::history),
    ("POST", "/api/jobs/cancel", routes::jobs::cancel),
];

/// Methods that never change state; every other method is behind the
/// same-origin guard.
const SAFE_METHODS: [&str; 3] = ["GET", "HEAD", "OPTIONS"];

/// Routes one request and renders the answer.
///
/// Every answer carries `Cache-Control: no-store` (personal data must not land
/// in the webview's disk cache) and is a plain answer: no ETag, `304` or gzip.
/// Order of checks, as in Go: route, method (`405` + `Allow`), same-origin
/// guard (`403`, only for state-changing methods), then the handler.
pub fn dispatch(state: &ApiState, req: Request) -> Response {
    finish(route(state, req))
}

fn route(state: &ApiState, mut req: Request) -> Response {
    let mut allowed: Vec<&str> = Vec::new();
    for (method, pattern, handler) in ROUTES {
        let Some(params) = matches(pattern, &req.path) else {
            continue;
        };
        if *method != req.method {
            if !allowed.contains(method) {
                allowed.push(method);
            }
            continue;
        }
        if !SAFE_METHODS.contains(method)
            && let Err(err) = state.origins.check(&req)
        {
            return err.into_response();
        }
        req.params = params;
        let outcome = catch_unwind(AssertUnwindSafe(|| handler(state, &req)));
        return match outcome {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => map_error(err, crate::shutdown::requested()).into_response(),
            Err(_) => {
                tracing::error!("api: handler panicked");
                ApiError::internal("internal error").into_response()
            }
        };
    }
    if allowed.is_empty() {
        return ApiError::not_found("ruta no encontrada").into_response();
    }
    ApiError::new(405, "método no permitido")
        .into_response()
        .with_header("Allow", &allowed.join(", "))
}

/// Go `apiErrorStatus`: an unexpected failure (500) while the process is
/// shutting down is a 503; explicit statuses are kept.
pub fn map_error(err: ApiError, shutting_down: bool) -> ApiError {
    if shutting_down && err.status == 500 {
        ApiError::unavailable(err.message)
    } else {
        err
    }
}

/// Forces `Cache-Control: no-store`, replacing whatever the handler set.
fn finish(mut response: Response) -> Response {
    response
        .headers
        .retain(|(name, _)| !name.eq_ignore_ascii_case("cache-control"));
    response.with_header("Cache-Control", "no-store")
}

/// `{x}` captures one non-empty path segment; the path must match exactly
/// (no trailing slash, no empty segments).
fn matches(pattern: &str, path: &str) -> Option<Vec<String>> {
    let want: Vec<&str> = pattern.strip_prefix('/')?.split('/').collect();
    let got: Vec<&str> = path.strip_prefix('/')?.split('/').collect();
    if want.len() != got.len() {
        return None;
    }
    let mut params = Vec::new();
    for (w, g) in want.iter().zip(&got) {
        if g.is_empty() {
            return None;
        }
        if w.starts_with('{') {
            params.push((*g).to_string());
        } else if w != g {
            return None;
        }
    }
    Some(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_capture_one_segment_and_match_exactly() {
        let p = "/api/facturas/{f}/portal/actualizar";
        assert_eq!(
            matches(p, "/api/facturas/ABC1/portal/actualizar"),
            Some(vec!["ABC1".to_string()])
        );
        assert_eq!(matches(p, "/api/facturas//portal/actualizar"), None);
        assert_eq!(matches(p, "/api/facturas/ABC1/portal/actualizar/"), None);
        assert_eq!(matches(p, "api/facturas/ABC1/portal/actualizar"), None);
        assert_eq!(matches(p, "/api/facturas/ABC1/portal"), None);
        assert_eq!(matches("/api/health", "/api/health"), Some(vec![]));
        assert_eq!(matches("/api/health", "/api/Health"), None);
        assert_eq!(matches("/api/health", "/api/health/"), None);
        assert_eq!(
            matches("/api/{a}/x/{b}", "/api/1/x/2"),
            Some(vec!["1".to_string(), "2".to_string()])
        );
    }

    #[test]
    fn shutdown_turns_an_unexpected_failure_into_503() {
        let internal = ApiError::internal("boom");
        assert_eq!(map_error(internal.clone(), false).status, 500);
        let mapped = map_error(internal, true);
        assert_eq!((mapped.status, mapped.message.as_str()), (503, "boom"));
        assert_eq!(map_error(ApiError::conflict("c"), true).status, 409);
        assert_eq!(map_error(ApiError::bad_request("b"), true).status, 400);
    }

    #[test]
    fn the_route_table_has_no_duplicate_method_and_pattern() {
        let mut seen = std::collections::HashSet::new();
        for (method, pattern, _) in ROUTES {
            assert!(seen.insert((*method, *pattern)), "{method} {pattern}");
        }
    }

    #[test]
    fn finish_replaces_any_cache_control() {
        let resp = finish(Response::new(200).with_header("cache-control", "no-cache"));
        assert_eq!(resp.header("Cache-Control"), Some("no-store"));
        assert_eq!(resp.headers.len(), 1);
    }
}
