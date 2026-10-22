//! Request-body helpers shared by the handlers: size limits and JSON decoding
//! with Go's rules (`cmd_serve.go: decodeJSONBody`).

use serde::de::DeserializeOwned;

use super::{errors::ApiError, request::Request};

/// Cap of the JSON bodies (Go `maxServeJSONBody`).
pub const MAX_JSON_BODY: usize = 64 << 10;

/// Largest CSV a manual upload may carry (Go `maxConvenioUpload`).
pub const MAX_UPLOAD_FILE: usize = 32 << 20;

/// Cap of a whole multipart upload body: the file plus 64 KiB of framing.
pub const MAX_UPLOAD_BODY: usize = MAX_UPLOAD_FILE + (64 << 10);

/// 413 message of an oversized JSON body.
pub const BODY_TOO_LARGE: &str = "cuerpo demasiado grande";

/// Why a JSON body was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    /// More than the limit.
    TooLarge,
    /// Not valid JSON for the target type (the decoder's message).
    Invalid(String),
    /// A second JSON value follows the first (only when trailing data is refused).
    Trailing,
}

impl JsonError {
    /// The generic answer: 413 `cuerpo demasiado grande`, otherwise 400
    /// `cuerpo invalido: <err>`.
    pub fn into_api_error(self) -> ApiError {
        match self {
            JsonError::TooLarge => ApiError::new(413, BODY_TOO_LARGE),
            JsonError::Invalid(err) => ApiError::bad_request(format!("cuerpo invalido: {err}")),
            JsonError::Trailing => ApiError::bad_request(
                "cuerpo invalido: el cuerpo debe contener un único objeto JSON",
            ),
        }
    }
}

/// What follows the first JSON value of a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trailing {
    /// Ignored, like Go's `json.Decoder.Decode` used once.
    Ignore,
    /// Refused ([`JsonError::Trailing`]); Go checks `Decode(new(any))` for `io.EOF`.
    Refuse,
}

/// Decodes `body` (at most `limit` bytes) into `T`.
///
/// Go matches object keys case-insensitively, so the top-level keys are
/// lower-cased before decoding: declare `T`'s fields in lower case (`cargue`,
/// `max_invoice_amount`) and `Cargue` / `CARGUE` / `cargue` all match. Of two
/// keys that collide once lower-cased the later one wins, as in Go. Nested
/// objects are left untouched. Use `#[serde(deny_unknown_fields)]` on `T` to
/// reject unknown fields.
pub fn parse_json<T: DeserializeOwned>(
    body: &[u8],
    limit: usize,
    trailing: Trailing,
) -> Result<T, JsonError> {
    if body.len() > limit {
        return Err(JsonError::TooLarge);
    }
    let mut stream = serde_json::Deserializer::from_slice(body).into_iter::<serde_json::Value>();
    let first = match stream.next() {
        Some(Ok(value)) => value,
        Some(Err(err)) => return Err(JsonError::Invalid(err.to_string())),
        None => return Err(JsonError::Invalid("EOF".into())),
    };
    if trailing == Trailing::Refuse && stream.next().is_some() {
        return Err(JsonError::Trailing);
    }
    serde_json::from_value(lowercase_keys(first)).map_err(|err| JsonError::Invalid(err.to_string()))
}

fn lowercase_keys(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| (key.to_lowercase(), value))
                .collect(),
        ),
        other => other,
    }
}

/// The request body as `T`, trailing data ignored (Go `decodeJSONBody`):
/// 413 `cuerpo demasiado grande` above `limit`, 400 `cuerpo invalido: <err>`.
pub fn read_json<T: DeserializeOwned>(req: &Request, limit: usize) -> Result<T, ApiError> {
    parse_json(&req.body, limit, Trailing::Ignore).map_err(JsonError::into_api_error)
}

/// [`read_json`] that also refuses a second JSON value after the first.
pub fn read_json_exact<T: DeserializeOwned>(req: &Request, limit: usize) -> Result<T, ApiError> {
    parse_json(&req.body, limit, Trailing::Refuse).map_err(JsonError::into_api_error)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Run {
        cargue: String,
        #[serde(default)]
        limit: i64,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Strict {
        factura: String,
    }

    fn req(body: &str) -> Request {
        Request::new("POST", "/x").with_body(body)
    }

    #[test]
    fn keys_match_case_insensitively() {
        for body in [
            r#"{"Cargue":"C1","Limit":3}"#,
            r#"{"cargue":"C1","limit":3}"#,
            r#"{"CARGUE":"C1","LiMiT":3}"#,
        ] {
            let run: Run = read_json(&req(body), MAX_JSON_BODY).expect(body);
            assert_eq!(
                run,
                Run {
                    cargue: "C1".into(),
                    limit: 3
                }
            );
        }
    }

    #[test]
    fn the_later_of_two_colliding_keys_wins() {
        let run: Run =
            read_json(&req(r#"{"cargue":"a","Cargue":"b"}"#), MAX_JSON_BODY).expect("decodes");
        assert_eq!(run.cargue, "b");
    }

    #[test]
    fn oversized_bodies_are_413_and_bad_json_is_400() {
        let big = format!(r#"{{"cargue":"{}"}}"#, "x".repeat(MAX_JSON_BODY));
        let err = read_json::<Run>(&req(&big), MAX_JSON_BODY).expect_err("too large");
        assert_eq!(
            (err.status, err.message.as_str()),
            (413, "cuerpo demasiado grande")
        );
        let err = read_json::<Run>(&req("{nope"), MAX_JSON_BODY).expect_err("invalid");
        assert_eq!(err.status, 400);
        assert!(
            err.message.starts_with("cuerpo invalido: "),
            "{}",
            err.message
        );
        let err = read_json::<Run>(&req(""), MAX_JSON_BODY).expect_err("empty");
        assert_eq!(
            (err.status, err.message.as_str()),
            (400, "cuerpo invalido: EOF")
        );
        let err = read_json::<Run>(&req(r#"{"limit":1}"#), MAX_JSON_BODY).expect_err("missing");
        assert_eq!(err.status, 400);
    }

    #[test]
    fn exactly_the_limit_is_accepted() {
        let body = r#"{"cargue":"C"}"#;
        assert!(read_json::<Run>(&req(body), body.len()).is_ok());
        assert_eq!(
            read_json::<Run>(&req(body), body.len() - 1)
                .expect_err("over")
                .status,
            413
        );
    }

    #[test]
    fn trailing_data_is_ignored_or_refused_on_demand() {
        let body = r#"{"cargue":"C"} {"cargue":"D"}"#;
        assert_eq!(
            read_json::<Run>(&req(body), MAX_JSON_BODY)
                .expect("first")
                .cargue,
            "C"
        );
        let err = read_json_exact::<Run>(&req(body), MAX_JSON_BODY).expect_err("trailing");
        assert_eq!(err.status, 400);
        assert!(err.message.starts_with("cuerpo invalido: "));
        assert!(read_json_exact::<Run>(&req("{\"cargue\":\"C\"}\n"), MAX_JSON_BODY).is_ok());
        assert_eq!(
            parse_json::<Run>(body.as_bytes(), MAX_JSON_BODY, Trailing::Refuse),
            Err(JsonError::Trailing)
        );
    }

    #[test]
    fn unknown_fields_are_refused_when_the_type_says_so() {
        let err = read_json_exact::<Strict>(&req(r#"{"Factura":"A1","extra":1}"#), MAX_JSON_BODY)
            .expect_err("unknown");
        assert_eq!(err.status, 400);
        assert!(err.message.contains("extra"), "{}", err.message);
        assert_eq!(
            read_json_exact::<Strict>(&req(r#"{"FACTURA":"A1"}"#), MAX_JSON_BODY).expect("ok"),
            Strict {
                factura: "A1".into()
            }
        );
    }
}
