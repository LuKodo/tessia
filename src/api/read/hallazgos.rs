//! The catalog of portal findings (Go `cmd_serve_hallazgos.go`):
//!
//! * `GET /api/hallazgos`: the catalog ordered by code;
//! * `PUT /api/hallazgos/{codigo}`: edits `genera_glosa`, `devolucion`
//!   (optional) and `descripcion` of an existing code.
//!
//! Both answer `Cache-Control: no-store`. A store failure is a generic 500:
//! the cause could carry connection details, so it only goes to the log.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::api::{
    ApiState,
    errors::{ApiError, ApiResult},
    request::{Request, Response},
};
use crate::phases::radicacion::portal::{CatalogItem, PortalStore};

/// Largest PUT body (it only carries two flags and a short text).
const MAX_BODY: usize = 64 << 10;
/// Longest description, counted in characters.
const MAX_DESCRIPTION_CHARS: usize = 2000;
/// Longest code in the URL.
const MAX_CODE_CHARS: usize = 100;

const STORE_FAILED: &str = "No se pudo acceder al catálogo de hallazgos.";

/// The JSON contract of the catalog endpoints; the dashboard UI is written
/// against these exact field names.
#[derive(Debug, Serialize)]
struct HallazgoView {
    codigo: String,
    detalle: String,
    genera_glosa: bool,
    devolucion: bool,
    descripcion: String,
}

impl From<CatalogItem> for HallazgoView {
    fn from(item: CatalogItem) -> Self {
        Self {
            detalle: item.detail().to_owned(),
            descripcion: item.description().to_owned(),
            codigo: item.code,
            genera_glosa: item.generates_glosa,
            devolucion: item.returns,
        }
    }
}

#[derive(Debug, Serialize)]
struct ListView {
    hallazgos: Vec<HallazgoView>,
    total: usize,
}

fn store_failed(op: &str, err: &dyn std::fmt::Display) -> ApiError {
    tracing::warn!(error = %err, "findings catalog {op} failed");
    ApiError::internal(STORE_FAILED)
}

pub fn list(state: &ApiState, _req: &Request) -> ApiResult {
    let items = PortalStore::new(state.pool.clone())
        .catalog()
        .map_err(|e| store_failed("list", &e))?;
    let hallazgos: Vec<HallazgoView> = items.into_iter().map(HallazgoView::from).collect();
    let total = hallazgos.len();
    Ok(Response::json(&ListView { hallazgos, total }).with_header("Cache-Control", "no-store"))
}

/// What the PUT body carries; `genera_glosa` and `devolucion` are `None` when
/// absent or `null`.
#[derive(Debug, Default, PartialEq, Eq)]
struct UpdateBody {
    genera_glosa: Option<bool>,
    devolucion: Option<bool>,
    descripcion: String,
}

fn not_valid_json() -> ApiError {
    ApiError::bad_request("El cuerpo debe ser un JSON válido con genera_glosa y descripcion.")
}

fn flag(value: &Value) -> Result<Option<bool>, ApiError> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(*b)),
        _ => Err(not_valid_json()),
    }
}

fn fill(body: &mut UpdateBody, fields: &Map<String, Value>) -> Result<(), ApiError> {
    for (key, value) in fields {
        // Go's decoder matches keys case-insensitively and rejects unknown ones.
        match key.to_ascii_lowercase().as_str() {
            "genera_glosa" => body.genera_glosa = flag(value)?,
            "devolucion" => body.devolucion = flag(value)?,
            "descripcion" => match value {
                Value::Null => body.descripcion.clear(),
                Value::String(text) => body.descripcion.clone_from(text),
                _ => return Err(not_valid_json()),
            },
            _ => return Err(not_valid_json()),
        }
    }
    Ok(())
}

/// Reads exactly one JSON object (`null` counts as an empty one, like Go).
fn parse_body(raw: &[u8]) -> Result<UpdateBody, ApiError> {
    if raw.len() > MAX_BODY {
        return Err(not_valid_json());
    }
    let mut values = serde_json::Deserializer::from_slice(raw).into_iter::<Value>();
    let first = match values.next() {
        Some(Ok(value)) => value,
        _ => return Err(not_valid_json()),
    };
    if values.next().is_some() {
        return Err(ApiError::bad_request(
            "El cuerpo debe contener un único objeto JSON.",
        ));
    }
    let mut body = UpdateBody::default();
    match first {
        Value::Null => {}
        Value::Object(fields) => fill(&mut body, &fields)?,
        _ => return Err(not_valid_json()),
    }
    Ok(body)
}

/// `^[A-Za-z0-9_.-]{1,100}$`.
fn valid_code(code: &str) -> bool {
    (1..=MAX_CODE_CHARS).contains(&code.len())
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

pub fn update(state: &ApiState, req: &Request) -> ApiResult {
    let code = req.param(0);
    if !valid_code(code) {
        return Err(ApiError::bad_request("Código de hallazgo inválido."));
    }
    let body = parse_body(&req.body)?;
    let Some(generates_glosa) = body.genera_glosa else {
        return Err(ApiError::bad_request("Falta el campo genera_glosa."));
    };
    let description = body.descripcion.trim();
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(ApiError::bad_request(
            "La descripción no puede superar 2000 caracteres.",
        ));
    }
    let item = PortalStore::new(state.pool.clone())
        .update_catalog(code, generates_glosa, body.devolucion, description)
        .map_err(|e| store_failed("update", &e))?
        .ok_or_else(|| ApiError::not_found("El hallazgo no existe."))?;
    Ok(Response::json(&HallazgoView::from(item)).with_header("Cache-Control", "no-store"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> Result<UpdateBody, String> {
        parse_body(text.as_bytes()).map_err(|e| e.message)
    }

    #[test]
    fn codes_follow_the_url_pattern() {
        assert!(valid_code("RVC012"));
        assert!(valid_code("a.b_c-9"));
        assert!(valid_code(&"A".repeat(100)));
        assert!(!valid_code(""));
        assert!(!valid_code(&"A".repeat(101)));
        assert!(!valid_code("a b"));
        assert!(!valid_code("ñ"));
        assert!(!valid_code("a/b"));
    }

    #[test]
    fn the_body_accepts_known_fields_in_any_case() {
        let parsed =
            body(r#"{"Genera_Glosa":true,"DEVOLUCION":false,"descripcion":" x "}"#).expect("valid");
        assert_eq!(
            parsed,
            UpdateBody {
                genera_glosa: Some(true),
                devolucion: Some(false),
                descripcion: " x ".into()
            }
        );
        let nulls =
            body(r#"{"genera_glosa":false,"devolucion":null,"descripcion":null}"#).expect("valid");
        assert_eq!(nulls.devolucion, None);
        assert_eq!(nulls.genera_glosa, Some(false));
        assert_eq!(
            body("null").expect("null is an empty object").genera_glosa,
            None
        );
        assert_eq!(body(" {} \n").expect("whitespace").genera_glosa, None);
    }

    #[test]
    fn a_bad_body_is_rejected_with_the_go_messages() {
        let invalid = "El cuerpo debe ser un JSON válido con genera_glosa y descripcion.";
        for text in [
            "",
            "{",
            "[]",
            "\"x\"",
            r#"{"extra":1}"#,
            r#"{"genera_glosa":"si"}"#,
            r#"{"genera_glosa":true,"descripcion":5}"#,
            r#"{"devolucion":1}"#,
        ] {
            assert_eq!(body(text), Err(invalid.to_owned()), "{text:?}");
        }
        let single = "El cuerpo debe contener un único objeto JSON.";
        for text in [r#"{"genera_glosa":true} {}"#, r#"{"genera_glosa":true}x"#] {
            assert_eq!(body(text), Err(single.to_owned()), "{text:?}");
        }
        let huge = format!(r#"{{"descripcion":"{}"}}"#, "a".repeat(MAX_BODY));
        assert_eq!(body(&huge), Err(invalid.to_owned()));
    }
}
