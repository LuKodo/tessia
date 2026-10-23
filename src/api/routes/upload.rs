//! `POST /api/convenio/upload`: a convenio CSV loaded by hand (Go:
//! `cmd_serve_convenio_upload.go`, spec gap 6).
//!
//! The request is `multipart/form-data` with exactly one file field `csv`
//! whose name ends in `.csv`. The CSV is read with the same reader `convenio
//! preparar` uses and refused when it has no valid rows or any malformed one.
//! A valid CSV becomes the cargue `Manual_<ts>_<hash12>` (S8): its bytes are
//! published to the bucket as `<id>.csv` and the cargue is registered with
//! origin `manual`, so `proceso run` picks it up like a downloaded one.
//!
//! The multipart body is parsed by hand: this route needs one in-memory body,
//! one field and Go's exact error messages, which does not justify a new
//! dependency.

use std::io::Cursor;

use chrono::Utc;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::api::{
    ApiError, ApiResult, ApiState,
    http::{MAX_UPLOAD_BODY, MAX_UPLOAD_FILE},
    request::{Request, Response},
};
use crate::domain::cargue::{LoadOrigin, load_context};
use crate::phases::convenio_preparar::csv::read_csv;
use crate::storage::Artifacts;
use crate::stores::artefactos::ArtefactosStore;
use crate::stores::cargues::{CarguesStore, IngestError, NewCargue};

pub const TOO_LARGE: &str = "El CSV supera el límite de 32 MiB.";
pub const BAD_MULTIPART: &str = "Solicitud multipart inválida.";
pub const ONE_CSV_ONLY: &str = "Selecciona un único archivo CSV.";
pub const INVALID_CSV: &str = "El CSV de convenio es inválido o no contiene registros válidos.";
pub const MALFORMED_ROWS: &str = "El CSV contiene filas con columnas incompletas o mal formadas.";
pub const STORE_CSV_FAILED: &str = "No se pudo guardar el CSV.";
pub const CREATE_FAILED: &str = "No se pudo crear el cargue.";
pub const ALREADY_EXISTS: &str = "Ya existe un cargue para esas fechas; no se reemplazó.";

pub fn post(state: &ApiState, req: &Request) -> ApiResult {
    if req.body.len() > MAX_UPLOAD_BODY {
        return Err(ApiError::new(413, TOO_LARGE));
    }
    let file = single_csv(req)?;
    if file.len() > MAX_UPLOAD_FILE {
        return Err(ApiError::new(413, TOO_LARGE));
    }
    let read = read_csv(Cursor::new(file)).map_err(|_| ApiError::bad_request(INVALID_CSV))?;
    if read.lines.is_empty() {
        return Err(ApiError::bad_request(INVALID_CSV));
    }
    if read.skipped > 0 {
        return Err(ApiError::bad_request(MALFORMED_ROWS));
    }

    let hash = hex::encode(Sha256::digest(file));
    // No dot in the timestamp: a cargue id is `[A-Za-z0-9_-]` (Go's id came
    // from sanitising the same name).
    let id = format!(
        "Manual_{}_{}",
        Utc::now().format("%Y%m%dT%H%M%S%9fZ"),
        &hash[..12]
    );
    let file_name = format!("{id}.csv");
    let info = load_context(&file_name);
    if info.id != id {
        tracing::error!("upload: generated cargue id is not folder-safe");
        return Err(ApiError::internal(CREATE_FAILED));
    }

    // Publish first: a failure then leaves only an unreferenced object, never a
    // cargue without its CSV.
    let artifacts = Artifacts::new(
        state.ctx.objects.clone(),
        ArtefactosStore::new(state.pool.clone()),
    );
    artifacts
        .publish_bytes(&id, &file_name, file)
        .map_err(|err| {
            tracing::warn!(error = %err, "upload: could not publish the CSV");
            ApiError::internal(STORE_CSV_FAILED)
        })?;
    let new = NewCargue::from_load(&info, &hash).with_origin(LoadOrigin::Manual);
    if let Err(err) = CarguesStore::new(state.pool.clone()).ingest(&new) {
        let _ = artifacts.delete(&id, &file_name);
        return Err(match err {
            IngestError::Superseded { .. } => ApiError::conflict(ALREADY_EXISTS),
            other => {
                tracing::warn!(error = %other, "upload: could not register the cargue");
                ApiError::internal(CREATE_FAILED)
            }
        });
    }
    Ok(Response::json(
        &json!({"ok": true, "cargue": id, "file_name": file_name}),
    ))
}

/// The bytes of the one `csv` file field, after Go's checks: exactly one file
/// part, named `csv`, whose file name ends in `.csv` (any case).
fn single_csv(req: &Request) -> Result<&[u8], ApiError> {
    let boundary = req
        .header("content-type")
        .and_then(boundary_of)
        .ok_or_else(|| ApiError::bad_request(BAD_MULTIPART))?;
    let parts =
        parse_parts(&req.body, &boundary).ok_or_else(|| ApiError::bad_request(BAD_MULTIPART))?;
    let mut files = parts.iter().filter(|p| p.filename.is_some());
    match (files.next(), files.next()) {
        (Some(part), None)
            if part.name == "csv" && part.filename.as_deref().is_some_and(has_csv_extension) =>
        {
            Ok(part.data)
        }
        _ => Err(ApiError::bad_request(ONE_CSV_ONLY)),
    }
}

/// Go `strings.EqualFold(filepath.Ext(name), ".csv")`.
fn has_csv_extension(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("csv"))
}

struct Part<'a> {
    name: String,
    /// Base name of the uploaded file; `None` for a plain value field.
    filename: Option<String>,
    data: &'a [u8],
}

/// The boundary of a `multipart/form-data` content type.
fn boundary_of(content_type: &str) -> Option<String> {
    let mut pieces = content_type.split(';');
    if !pieces
        .next()?
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
    {
        return None;
    }
    pieces
        .filter_map(|piece| piece.split_once('='))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case("boundary"))
        .map(|(_, value)| value.trim().trim_matches('"').to_owned())
        .filter(|value| !value.is_empty() && value.len() <= 200)
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let first = *needle.first()?;
    let mut at = from;
    while at + needle.len() <= haystack.len() {
        let offset = haystack[at..=haystack.len() - needle.len()]
            .iter()
            .position(|&b| b == first)?;
        at += offset;
        if haystack[at..].starts_with(needle) {
            return Some(at);
        }
        at += 1;
    }
    None
}

/// Splits a multipart body into its parts (CRLF framing, as browsers send it).
/// `None` when the body is not well formed.
fn parse_parts<'a>(body: &'a [u8], boundary: &str) -> Option<Vec<Part<'a>>> {
    let delimiter = format!("--{boundary}").into_bytes();
    let next_delimiter = format!("\r\n--{boundary}").into_bytes();
    let mut parts = Vec::new();
    // The preamble before the first delimiter is ignored.
    let mut at = find(body, &delimiter, 0)? + delimiter.len();
    loop {
        if body[at..].starts_with(b"--") {
            return Some(parts);
        }
        // Optional transport padding, then the line break after the delimiter.
        while matches!(body.get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
        if !body[at..].starts_with(b"\r\n") {
            return None;
        }
        at += 2;
        let headers_end = if body[at..].starts_with(b"\r\n") {
            at // a part without headers
        } else {
            find(body, b"\r\n\r\n", at)? + 2
        };
        let headers = std::str::from_utf8(&body[at..headers_end]).ok()?;
        let data_start = headers_end + 2;
        // The line break that ends the headers may also be the one before the
        // delimiter (an empty value), so the search starts at `headers_end`.
        let data_end = find(body, &next_delimiter, headers_end)?;
        let (name, filename) = disposition(headers);
        parts.push(Part {
            name: name.unwrap_or_default(),
            filename: filename.filter(|f| !f.is_empty()),
            data: body.get(data_start..data_end).unwrap_or_default(),
        });
        at = data_end + next_delimiter.len();
    }
}

/// `name` and base `filename` of a part's `Content-Disposition` header.
fn disposition(headers: &str) -> (Option<String>, Option<String>) {
    for line in headers.split("\r\n") {
        let Some((header, value)) = line.split_once(':') else {
            continue;
        };
        if !header.trim().eq_ignore_ascii_case("content-disposition") {
            continue;
        }
        let mut name = None;
        let mut filename = None;
        for (key, parameter) in parameters(value) {
            match key.as_str() {
                "name" => name = Some(parameter),
                "filename" => filename = Some(base_name(&parameter).to_owned()),
                _ => {}
            }
        }
        return (name, filename);
    }
    (None, None)
}

/// `; key="quoted \" value"; other=token` parameters, keys lower-cased.
fn parameters(value: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = value.chars().peekable();
    // Skip the disposition type (`form-data`).
    for c in chars.by_ref() {
        if c == ';' {
            break;
        }
    }
    loop {
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c == ';' {
                break;
            }
            key.push(c);
            chars.next();
        }
        if chars.next() != Some('=') {
            break;
        }
        let mut text = String::new();
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    // Only `\"` and `\\` are escapes; browsers on Windows send
                    // a bare backslash in a path.
                    '\\' => match chars.peek() {
                        Some(&escaped @ ('"' | '\\')) => {
                            text.push(escaped);
                            chars.next();
                        }
                        _ => text.push('\\'),
                    },
                    '"' => break,
                    other => text.push(other),
                }
            }
            for c in chars.by_ref() {
                if c == ';' {
                    break;
                }
            }
        } else {
            for c in chars.by_ref() {
                if c == ';' {
                    break;
                }
                text.push(c);
            }
            text = text.trim().to_owned();
        }
        out.push((key.trim().to_ascii_lowercase(), text));
    }
    out
}

/// Go `filepath.Base` on both separators (browsers may send a full path).
fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(boundary: &str, parts: &[(&str, Option<&str>, &str)]) -> Vec<u8> {
        let mut out = String::new();
        for (name, file, data) in parts {
            out.push_str(&format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\""
            ));
            if let Some(file) = file {
                out.push_str(&format!("; filename=\"{file}\"\r\nContent-Type: text/csv"));
            }
            out.push_str(&format!("\r\n\r\n{data}\r\n"));
        }
        out.push_str(&format!("--{boundary}--\r\n"));
        out.into_bytes()
    }

    #[test]
    fn the_boundary_is_read_from_the_content_type() {
        assert_eq!(
            boundary_of("multipart/form-data; boundary=----X1").as_deref(),
            Some("----X1")
        );
        assert_eq!(
            boundary_of("Multipart/Form-Data;charset=utf-8; BOUNDARY=\"a b\"").as_deref(),
            Some("a b")
        );
        assert_eq!(boundary_of("multipart/form-data"), None);
        assert_eq!(boundary_of("multipart/form-data; boundary="), None);
        assert_eq!(boundary_of("application/json; boundary=x"), None);
        assert_eq!(boundary_of("multipart/mixed; boundary=x"), None);
    }

    #[test]
    fn parts_keep_their_bytes_exactly() {
        let data = "a;b\r\n--not-a-boundary\r\n\r\nz\n";
        let raw = body("B", &[("note", None, "hi"), ("csv", Some("a.csv"), data)]);
        let parts = parse_parts(&raw, "B").expect("parses");
        assert_eq!(parts.len(), 2);
        assert_eq!(
            (parts[0].name.as_str(), parts[0].filename.clone()),
            ("note", None)
        );
        assert_eq!(parts[0].data, b"hi");
        assert_eq!(parts[1].filename.as_deref(), Some("a.csv"));
        assert_eq!(parts[1].data, data.as_bytes());
    }

    #[test]
    fn a_preamble_and_an_epilogue_are_ignored() {
        let mut raw = b"ignore me\r\n".to_vec();
        raw.extend(body("B", &[("csv", Some("a.csv"), "x")]));
        raw.extend(b"trailing junk");
        let parts = parse_parts(&raw, "B").expect("parses");
        assert_eq!(parts[0].data, b"x");
    }

    #[test]
    fn malformed_bodies_are_refused() {
        assert!(parse_parts(b"", "B").is_none());
        assert!(parse_parts(b"no delimiter here", "B").is_none());
        assert!(
            parse_parts(
                b"--B\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nx",
                "B"
            )
            .is_none()
        );
        assert!(parse_parts(b"--B junk\r\n", "B").is_none());
        let empty_value = parse_parts(
            b"--B\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n\r\n--B--",
            "B",
        )
        .expect("empty value");
        assert_eq!(empty_value[0].data, b"");
        let no_headers = parse_parts(b"--B\r\n\r\nx\r\n--B--", "B").expect("no headers");
        assert_eq!(
            (no_headers[0].name.as_str(), no_headers[0].data),
            ("", &b"x"[..])
        );
        let empty = parse_parts(b"--B--\r\n", "B").expect("empty form");
        assert!(empty.is_empty());
    }

    #[test]
    fn file_names_lose_their_directories_and_unescape_quotes() {
        let raw = body("B", &[("csv", Some("C:\\Users\\x\\conv.CSV"), "x")]);
        let parts = parse_parts(&raw, "B").expect("parses");
        assert_eq!(parts[0].filename.as_deref(), Some("conv.CSV"));
        let (name, file) = disposition(
            "Content-Disposition: form-data; name=\"c\\\"sv\"; filename=\"/tmp/a b.csv\"",
        );
        assert_eq!(name.as_deref(), Some("c\"sv"));
        assert_eq!(file.as_deref(), Some("a b.csv"));
        let (name, file) = disposition("content-disposition: form-data; name=csv");
        assert_eq!((name.as_deref(), file), (Some("csv"), None));
    }

    #[test]
    fn extensions_follow_go_ext() {
        assert!(has_csv_extension("a.csv"));
        assert!(has_csv_extension("a.CsV"));
        assert!(has_csv_extension(".csv"));
        assert!(!has_csv_extension("csv"));
        assert!(!has_csv_extension("a.csv.txt"));
        assert!(!has_csv_extension("a.xlsx"));
    }

    #[test]
    fn find_locates_needles_at_the_edges() {
        assert_eq!(find(b"abcabc", b"abc", 1), Some(3));
        assert_eq!(find(b"abc", b"abc", 0), Some(0));
        assert_eq!(find(b"ab", b"abc", 0), None);
        assert_eq!(find(b"abc", b"c", 3), None);
    }
}
