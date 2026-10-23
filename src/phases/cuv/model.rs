//! SISPRO's validation response, how Java read it and what it wrote
//! (Go `internal/cuv/cuv.go`): `RespuestaValidacion` parsing like Gson,
//! `recibirRepuestaCUV` (`interpret`), the rejection summary, the
//! `CargarFevRips` payload and the régimen of the invoice.

use base64::Engine;
use serde::Deserialize;

use super::jsontree::{java_trim, parse_ordered};
use crate::util::gson::{GsonOptions, Json, to_string};

/// `Modelo/Ministerio/ResultadosValidacion` (field order matters).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationResult {
    pub clase: Option<String>,
    pub codigo: Option<String>,
    pub descripcion: Option<String>,
    pub observaciones: Option<String>,
    pub path_fuente: Option<String>,
    pub fuente: Option<String>,
}

/// `Modelo/Ministerio/RespuestaValidacion` (field order matters).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Response {
    pub result_state: bool,
    pub proceso_id: i32,
    pub num_factura: Option<String>,
    pub codigo_unico_validacion: Option<String>,
    pub codigo_unico_validacion_to_show: Option<String>,
    pub fecha_radicacion: Option<String>,
    pub ruta_archivos: Option<String>,
    /// `None` is a JSON `null` / missing member; a `None` element is a `null` item.
    pub resultados: Option<Vec<Option<ValidationResult>>>,
}

fn opt_string(value: &Option<String>) -> Json {
    value.clone().map_or(Json::Null, Json::String)
}

impl ValidationResult {
    fn to_json(&self) -> Json {
        Json::object([
            ("Clase", opt_string(&self.clase)),
            ("Codigo", opt_string(&self.codigo)),
            ("Descripcion", opt_string(&self.descripcion)),
            ("Observaciones", opt_string(&self.observaciones)),
            ("PathFuente", opt_string(&self.path_fuente)),
            ("Fuente", opt_string(&self.fuente)),
        ])
    }
}

impl Response {
    /// Gson `serializeNulls + disableHtmlEscaping + prettyPrinting` of the
    /// model, in field order: the content of `<F>.txt` before the sweep.
    pub fn encode(&self) -> String {
        let results = match &self.resultados {
            None => Json::Null,
            Some(items) => Json::Array(
                items
                    .iter()
                    .map(|r| r.as_ref().map_or(Json::Null, ValidationResult::to_json))
                    .collect(),
            ),
        };
        let doc = Json::object([
            ("ResultState", Json::Bool(self.result_state)),
            ("ProcesoId", Json::Int(i64::from(self.proceso_id))),
            ("NumFactura", opt_string(&self.num_factura)),
            (
                "CodigoUnicoValidacion",
                opt_string(&self.codigo_unico_validacion),
            ),
            (
                "CodigoUnicoValidacionToShow",
                opt_string(&self.codigo_unico_validacion_to_show),
            ),
            ("FechaRadicacion", opt_string(&self.fecha_radicacion)),
            ("RutaArchivos", opt_string(&self.ruta_archivos)),
            ("ResultadosValidacion", results),
        ]);
        let options = GsonOptions {
            serialize_nulls: true,
            ..GsonOptions::default()
        };
        to_string(&doc, &options)
    }

    /// `gson.fromJson(body, RespuestaValidacion.class)`: member names are
    /// case sensitive, unknown members are ignored, strings also accept numbers
    /// and booleans, ints and booleans also accept strings.
    pub fn parse(body: &[u8]) -> Result<Response, String> {
        let trimmed = body.trim_ascii();
        if trimmed.first() != Some(&b'{') {
            return Err("cuv: respuesta no es un objeto JSON".into());
        }
        let Json::Object(members) = parse_ordered(trimmed)? else {
            return Err("cuv: respuesta no es un objeto JSON".into());
        };
        let mut response = Response::default();
        for (key, value) in &members {
            match key.as_str() {
                "ResultState" => {
                    response.result_state = gson_bool(value, response.result_state)
                        .map_err(|e| format!("cuv: ResultState: {e}"))?;
                }
                "ProcesoId" => {
                    response.proceso_id = gson_int(value, response.proceso_id)
                        .map_err(|e| format!("cuv: ProcesoId: {e}"))?;
                }
                "NumFactura" => response.num_factura = gson_field(key, value)?,
                "CodigoUnicoValidacion" => {
                    response.codigo_unico_validacion = gson_field(key, value)?;
                }
                "CodigoUnicoValidacionToShow" => {
                    response.codigo_unico_validacion_to_show = gson_field(key, value)?;
                }
                "FechaRadicacion" => response.fecha_radicacion = gson_field(key, value)?,
                "RutaArchivos" => response.ruta_archivos = gson_field(key, value)?,
                "ResultadosValidacion" => response.resultados = parse_results(value)?,
                _ => {}
            }
        }
        Ok(response)
    }
}

fn gson_field(key: &str, value: &Json) -> Result<Option<String>, String> {
    gson_string(value).map_err(|e| format!("cuv: {key}: {e}"))
}

fn parse_results(value: &Json) -> Result<Option<Vec<Option<ValidationResult>>>, String> {
    let items = match value {
        Json::Null => return Ok(None),
        Json::Array(items) => items,
        _ => return Err("cuv: ResultadosValidacion: se esperaba una lista".into()),
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let members = match item {
            Json::Null => {
                out.push(None);
                continue;
            }
            Json::Object(members) => members,
            _ => return Err("cuv: respuesta no es un objeto JSON".into()),
        };
        let mut result = ValidationResult::default();
        for (key, value) in members {
            let slot = match key.as_str() {
                "Clase" => &mut result.clase,
                "Codigo" => &mut result.codigo,
                "Descripcion" => &mut result.descripcion,
                "Observaciones" => &mut result.observaciones,
                "PathFuente" => &mut result.path_fuente,
                "Fuente" => &mut result.fuente,
                _ => continue,
            };
            *slot = gson_field(key, value)?;
        }
        out.push(Some(result));
    }
    Ok(Some(out))
}

fn gson_string(value: &Json) -> Result<Option<String>, String> {
    match value {
        Json::Null => Ok(None),
        Json::String(s) => Ok(Some(s.clone())),
        Json::Number(literal) => Ok(Some(literal.clone())),
        Json::Bool(b) => Ok(Some(b.to_string())),
        _ => Err("se esperaba texto".into()),
    }
}

fn gson_bool(value: &Json, default: bool) -> Result<bool, String> {
    match value {
        Json::Null => Ok(default),
        Json::Bool(b) => Ok(*b),
        Json::String(s) => Ok(s.eq_ignore_ascii_case("true")),
        _ => Err("se esperaba booleano".into()),
    }
}

fn gson_int(value: &Json, default: i32) -> Result<i32, String> {
    match value {
        Json::Null => Ok(default),
        Json::String(text) | Json::Number(text) => int_from_text(text),
        _ => Err("se esperaba entero".into()),
    }
}

fn int_from_text(text: &str) -> Result<i32, String> {
    if let Ok(n) = text.parse::<i32>() {
        return Ok(n);
    }
    let invalid = || format!("entero invalido {text:?}");
    let float: f64 = text.parse().map_err(|_| invalid())?;
    let truncated = float as i32;
    if f64::from(truncated) == float {
        Ok(truncated)
    } else {
        Err(invalid())
    }
}

/// What `recibirRepuestaCUV` did with one HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interpreted {
    /// The file written next to `<F>.json`; `None` = none. `<F>.txt` (200/201),
    /// `<F>_ERROR.txt` (400) or `<F>_RESPUESTA_NO_CORRESPONDE.txt`.
    pub file_name: Option<String>,
    pub content: Vec<u8>,
    /// The text Java showed in the transaction table.
    pub summary: String,
    /// Java returned false: nothing usable was written.
    pub failed: bool,
    /// The parsed body (default when it could not be parsed).
    pub response: Response,
}

const SUMMARY_EXCEPTION: &str = "Error 500 al consumir el servicio";

fn failure() -> Interpreted {
    Interpreted {
        file_name: None,
        content: Vec::new(),
        summary: SUMMARY_EXCEPTION.into(),
        failed: true,
        response: Response::default(),
    }
}

/// `recibirRepuestaCUV` for `invoice` (no dash) and the HTTP status and body of
/// `CargarFevRips`. Java's NullPointerExceptions (a missing list on 400, a
/// result without `Clase` on 200) are kept: they landed in the catch block, so
/// nothing is written.
pub fn interpret(invoice: &str, status: u16, body: &[u8]) -> Interpreted {
    let Ok(response) = Response::parse(body) else {
        return failure();
    };
    match status {
        200 | 201 => {
            let mut summary = String::new();
            if let Some(items) = &response.resultados {
                let mut codes = Vec::new();
                for item in items {
                    let Some(clase) = item.as_ref().and_then(|r| r.clase.as_deref()) else {
                        return failure();
                    };
                    if clase == "RECHAZADO" {
                        let codigo = item.as_ref().and_then(|r| r.codigo.as_deref());
                        codes.push(format!("Codigo:{}", codigo.unwrap_or("")));
                    }
                }
                summary = if codes.is_empty() {
                    "CUV exitoso".into()
                } else {
                    codes.join("|")
                };
            }
            save(invoice, response, "", &summary)
        }
        400 => {
            if !all_items_present(&response) {
                return failure();
            }
            save(invoice, response, "_ERROR", "Error 400")
        }
        other => {
            if !all_items_present(&response) {
                return failure();
            }
            Interpreted {
                file_name: None,
                content: Vec::new(),
                summary: "Error general".into(),
                failed: other >= 500,
                response,
            }
        }
    }
}

/// The list exists and has no `null` element.
fn all_items_present(response: &Response) -> bool {
    response
        .resultados
        .as_ref()
        .is_some_and(|items| items.iter().all(Option::is_some))
}

/// Normalization of an invoice number the way Java compared them.
fn normalize_invoice(s: &str) -> String {
    java_trim(s).replace('-', "").to_uppercase()
}

/// `guardarRespuestaSiCorresponde`.
fn save(invoice: &str, response: Response, suffix: &str, summary: &str) -> Interpreted {
    let content = response.encode();
    let received = response.num_factura.clone().unwrap_or_default();
    let got = normalize_invoice(&received);
    if got.is_empty() || got == normalize_invoice(invoice) {
        return Interpreted {
            file_name: Some(format!("{invoice}{suffix}.txt")),
            content: content.into_bytes(),
            summary: summary.into(),
            failed: false,
            response,
        };
    }
    let diagnostic = format!(
        "Respuesta no corresponde a la factura local.\nFactura local: {invoice}\nFactura respuesta Ministerio: {received}\nRespuesta:\n{content}"
    );
    Interpreted {
        file_name: Some(format!("{invoice}_RESPUESTA_NO_CORRESPONDE.txt")),
        content: diagnostic.into_bytes(),
        summary: format!("Respuesta no corresponde: {received}"),
        failed: true,
        response,
    }
}

/// One invoice can be rejected hundreds of times (the same rule on every
/// line): only this many rejections are spelled out.
const MAX_REJECTIONS_LISTED: usize = 5;

/// Why SISPRO rejected an invoice: one `"Clase" - "Codigo" - "PathFuente"` per
/// `RECHAZADO` result joined by ` | ` (an empty or missing `PathFuente` is left
/// out; more than five end in ` | +N mas`). Empty when the response rejects
/// nothing.
pub fn rejection_summary(response: &Response) -> String {
    let mut parts: Vec<String> = response
        .resultados
        .iter()
        .flatten()
        .flatten()
        .filter(|r| r.clase.as_deref() == Some("RECHAZADO"))
        .map(|r| {
            let mut part = format!(
                "\"RECHAZADO\" - \"{}\"",
                r.codigo.as_deref().unwrap_or_default()
            );
            if let Some(path) = r.path_fuente.as_deref().filter(|p| !p.is_empty()) {
                part.push_str(&format!(" - \"{path}\""));
            }
            part
        })
        .collect();
    if parts.len() > MAX_REJECTIONS_LISTED {
        let more = parts.len() - MAX_REJECTIONS_LISTED;
        parts.truncate(MAX_REJECTIONS_LISTED);
        parts.push(format!("+{more} mas"));
    }
    parts.join(" | ")
}

/// The `CargarFevRips` body `{"rips": …, "xmlFevFile": …}`
/// (`Modelo/Estructura/Root`) from the `<F>.json` bytes and the FEV XML.
pub fn build_payload(rips_json: &[u8], xml: &[u8]) -> Result<Vec<u8>, String> {
    let rips = rips_json.trim_ascii();
    let valid_object = rips.first() == Some(&b'{')
        && serde_json::from_slice::<serde::de::IgnoredAny>(rips).is_ok();
    if !valid_object {
        return Err("cuv: el JSON RIPS no es un objeto JSON valido".into());
    }
    if xml.is_empty() {
        return Err("cuv: XML vacio".into());
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(xml);
    let mut payload = Vec::with_capacity(rips.len() + encoded.len() + 32);
    payload.extend_from_slice(br#"{"rips":"#);
    payload.extend_from_slice(rips);
    payload.extend_from_slice(br#","xmlFevFile":""#);
    payload.extend_from_slice(encoded.as_bytes());
    payload.extend_from_slice(br#""}"#);
    Ok(payload)
}

#[derive(Deserialize)]
struct RipsHead {
    usuarios: Option<Vec<Option<RipsUser>>>,
}

#[derive(Deserialize)]
struct RipsUser {
    #[serde(rename = "tipoUsuario")]
    tipo_usuario: Option<String>,
}

/// The régimen of the first user of `<F>.json`, normalized the way
/// `obtenerRegimenUsuarioPorFactura` did (`01` contributivo, `04` subsidiado).
/// Empty when it cannot be read.
pub fn regimen(rips_json: &[u8]) -> String {
    let Ok(head) = serde_json::from_slice::<RipsHead>(rips_json) else {
        return String::new();
    };
    let tipo = head
        .usuarios
        .and_then(|users| users.into_iter().next())
        .flatten()
        .and_then(|user| user.tipo_usuario)
        .unwrap_or_default();
    normalize_regimen(&tipo)
}

fn normalize_regimen(raw: &str) -> String {
    let mut s = java_trim(raw).to_owned();
    for (from, to) in [
        ("EPS048", "01"),
        ("EPSS48", "04"),
        ("ESS207", "04"),
        ("ESSC07", "01"),
    ] {
        s = s.replace(from, to);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clase(clase: &str, codigo: &str, path: Option<&str>) -> Option<ValidationResult> {
        Some(ValidationResult {
            clase: Some(clase.into()),
            codigo: Some(codigo.into()),
            path_fuente: path.map(str::to_owned),
            ..ValidationResult::default()
        })
    }

    #[test]
    fn parses_like_gson_text_for_numbers_and_numbers_for_text() {
        let r = Response::parse(
            br#"{"ResultState":"TRUE","ProcesoId":"77","NumFactura":5,"FechaRadicacion":1.50,"CodigoUnicoValidacion":false,"Extra":{"x":1}}"#,
        )
        .expect("parse");
        assert!(r.result_state);
        assert_eq!(r.proceso_id, 77);
        assert_eq!(r.num_factura.as_deref(), Some("5"));
        assert_eq!(r.fecha_radicacion.as_deref(), Some("1.50"));
        assert_eq!(r.codigo_unico_validacion.as_deref(), Some("false"));
        assert_eq!(r.resultados, None);
        let r = Response::parse(br#"{"ProcesoId":12.0}"#).expect("float int");
        assert_eq!(r.proceso_id, 12);
    }

    #[test]
    fn rejects_what_gson_rejects() {
        for bad in [
            &b""[..],
            b"[]",
            b"not json",
            br#"{"ResultState":[1]}"#,
            br#"{"ProcesoId":true}"#,
            br#"{"ProcesoId":1.5}"#,
            br#"{"ProcesoId":3000000000}"#,
            br#"{"NumFactura":{"a":1}}"#,
            br#"{"ResultadosValidacion":{}}"#,
            br#"{"ResultadosValidacion":[1]}"#,
            br#"{"ResultadosValidacion":[{"Clase":[]}]}"#,
            br#"{"a":1} trailing"#,
        ] {
            assert!(
                Response::parse(bad).is_err(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        assert!(Response::parse(b"  {\"ResultState\":true}\n").is_ok());
    }

    #[test]
    fn interpret_follows_recibir_repuesta_cuv() {
        let ok = br#"{"ResultState":true,"NumFactura":"fvs-1","CodigoUnicoValidacion":"abc","ResultadosValidacion":[{"Clase":"NOTIFICACION","Codigo":"X"}]}"#;
        for status in [200, 201] {
            let o = interpret("FVS1", status, ok);
            assert_eq!(o.file_name.as_deref(), Some("FVS1.txt"));
            assert_eq!(o.summary, "CUV exitoso");
            assert!(!o.failed);
        }
        let rejected = br#"{"NumFactura":"FVS1","ResultadosValidacion":[{"Clase":"RECHAZADO","Codigo":"RVG01"},{"Clase":"RECHAZADO","Codigo":"RVG02"}]}"#;
        let o = interpret("FVS1", 200, rejected);
        assert_eq!(o.summary, "Codigo:RVG01|Codigo:RVG02");
        let o = interpret("FVS1", 400, rejected);
        assert_eq!(o.file_name.as_deref(), Some("FVS1_ERROR.txt"));
        assert_eq!(o.summary, "Error 400");
        // A 200 without a result list is a success summary of "" (Java: no loop).
        let o = interpret("FVS1", 200, br#"{"NumFactura":"FVS1"}"#);
        assert_eq!(o.file_name.as_deref(), Some("FVS1.txt"));
        assert_eq!(o.summary, "");
    }

    #[test]
    fn interpret_keeps_the_java_null_pointer_cases() {
        let no_clase = br#"{"ResultadosValidacion":[{"Codigo":"X"}]}"#;
        let o = interpret("F1", 200, no_clase);
        assert!(o.file_name.is_none() && o.failed);
        assert_eq!(o.summary, "Error 500 al consumir el servicio");
        let o = interpret("F1", 400, br#"{"NumFactura":"F1"}"#);
        assert!(o.file_name.is_none() && o.failed);
        let o = interpret("F1", 400, br#"{"ResultadosValidacion":[null]}"#);
        assert!(o.file_name.is_none() && o.failed);
        let o = interpret("F1", 200, b"<html>");
        assert!(o.file_name.is_none() && o.failed);
    }

    #[test]
    fn other_statuses_write_nothing_and_only_5xx_is_a_failure() {
        let body = br#"{"ResultadosValidacion":[]}"#;
        for (status, failed) in [(404, false), (409, false), (500, true), (503, true)] {
            let o = interpret("F1", status, body);
            assert!(o.file_name.is_none());
            assert_eq!(o.summary, "Error general");
            assert_eq!(o.failed, failed, "status {status}");
        }
        let o = interpret("F1", 500, br#"{"NumFactura":"F1"}"#);
        assert_eq!(o.summary, "Error 500 al consumir el servicio");
        assert!(o.failed);
    }

    #[test]
    fn a_response_for_another_invoice_becomes_the_diagnostic_file() {
        let body = br#"{"NumFactura":" fvs-2 ","ResultadosValidacion":[]}"#;
        let o = interpret("FVS1", 200, body);
        assert_eq!(
            o.file_name.as_deref(),
            Some("FVS1_RESPUESTA_NO_CORRESPONDE.txt")
        );
        assert_eq!(o.summary, "Respuesta no corresponde:  fvs-2 ");
        assert!(o.failed);
        let text = String::from_utf8(o.content).expect("utf8");
        assert!(text.starts_with(
            "Respuesta no corresponde a la factura local.\nFactura local: FVS1\nFactura respuesta Ministerio:  fvs-2 \nRespuesta:\n{\n"
        ));
        // The same invoice written differently, or no NumFactura, is fine.
        assert!(
            interpret("FVS1", 200, br#"{"NumFactura":" fvs-1"}"#).file_name
                == Some("FVS1.txt".into())
        );
        assert!(
            interpret("FVS1", 200, br#"{"NumFactura":"  "}"#).file_name == Some("FVS1.txt".into())
        );
    }

    #[test]
    fn rejection_summary_lists_five_and_counts_the_rest() {
        let mut items = vec![
            clase("RECHAZADO", "RVG01", Some("Rips.usuarios[0].x")),
            clase("NOTIFICACION", "RVC1", Some("ignored")),
            clase("RECHAZADO", "RVG02", Some("")),
            None,
            clase("RECHAZADO", "RVG03", None),
        ];
        let response = Response {
            resultados: Some(items.clone()),
            ..Response::default()
        };
        assert_eq!(
            rejection_summary(&response),
            "\"RECHAZADO\" - \"RVG01\" - \"Rips.usuarios[0].x\" | \"RECHAZADO\" - \"RVG02\" | \"RECHAZADO\" - \"RVG03\""
        );
        for n in 0..4 {
            items.push(clase("RECHAZADO", &format!("R{n}"), None));
        }
        let response = Response {
            resultados: Some(items),
            ..Response::default()
        };
        let text = rejection_summary(&response);
        assert!(
            text.ends_with(" | \"RECHAZADO\" - \"R0\" | \"RECHAZADO\" - \"R1\" | +2 mas"),
            "{text}"
        );
        assert_eq!(rejection_summary(&Response::default()), "");
    }

    #[test]
    fn payload_embeds_the_rips_bytes_and_the_base64_xml() {
        let payload = build_payload(b"  {\"a\": 1}\n", b"<x/>").expect("payload");
        assert_eq!(
            String::from_utf8(payload).expect("utf8"),
            "{\"rips\":{\"a\": 1},\"xmlFevFile\":\"PHgvPg==\"}"
        );
        assert_eq!(
            build_payload(b"[]", b"<x/>").expect_err("array"),
            "cuv: el JSON RIPS no es un objeto JSON valido"
        );
        assert!(build_payload(b"{\"a\":", b"<x/>").is_err());
        assert_eq!(
            build_payload(b"{}", b"").expect_err("empty"),
            "cuv: XML vacio"
        );
    }

    #[test]
    fn regimen_reads_the_first_user_with_the_java_replacements() {
        let rips = |tipo: &str| {
            format!(r#"{{"usuarios":[{{"tipoUsuario":"{tipo}"}},{{"tipoUsuario":"04"}}]}}"#)
        };
        assert_eq!(regimen(rips("01").as_bytes()), "01");
        assert_eq!(regimen(rips(" EPS048 ").as_bytes()), "01");
        assert_eq!(regimen(rips("EPSS48").as_bytes()), "04");
        assert_eq!(regimen(rips("ESS207").as_bytes()), "04");
        assert_eq!(regimen(rips("ESSC07").as_bytes()), "01");
        assert_eq!(regimen(b"{}"), "");
        assert_eq!(regimen(br#"{"usuarios":[]}"#), "");
        assert_eq!(regimen(br#"{"usuarios":[{"tipoUsuario":4}]}"#), "");
        assert_eq!(regimen(b"nope"), "");
    }
}
