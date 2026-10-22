//! Route table and dispatch. Fallbacks answer 404 `ruta no encontrada` and 405
//! `método no permitido` (with `Allow`), as Go's router does.

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

pub fn dispatch(state: &ApiState, mut req: Request) -> Response {
    let mut allowed: Vec<&str> = Vec::new();
    for (method, pattern, handler) in ROUTES {
        let Some(params) = matches(pattern, &req.path) else {
            continue;
        };
        if *method != req.method {
            allowed.push(method);
            continue;
        }
        req.params = params;
        return match handler(state, &req) {
            Ok(response) => response,
            Err(err) => err.into_response(),
        };
    }
    if allowed.is_empty() {
        return ApiError::not_found("ruta no encontrada").into_response();
    }
    ApiError::new(405, "método no permitido")
        .into_response()
        .with_header("Allow", &allowed.join(", "))
}

fn matches(pattern: &str, path: &str) -> Option<Vec<String>> {
    let want: Vec<&str> = pattern.trim_matches('/').split('/').collect();
    let got: Vec<&str> = path.trim_matches('/').split('/').collect();
    if want.len() != got.len() {
        return None;
    }
    let mut params = Vec::new();
    for (w, g) in want.iter().zip(&got) {
        if w.starts_with('{') {
            if g.is_empty() {
                return None;
            }
            params.push((*g).to_string());
        } else if w != g {
            return None;
        }
    }
    Some(params)
}
