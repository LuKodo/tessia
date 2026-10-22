//! Pure rules that classify Mutual Ser responses (Go `internal/radicacion/rules.go`).
//!
//! Every rule is a substring match on upper-cased text. The Spanish markers
//! are the portal's own messages and a stored contract: keep them verbatim.

use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::Value;

use crate::domain::radicacion::Step;

/// Upper-cases and strips the accents the legacy rules normalize.
fn upper(s: &str) -> String {
    s.to_uppercase()
        .replace('Á', "A")
        .replace('É', "E")
        .replace('Í', "I")
        .replace('Ó', "O")
        .replace('Ú', "U")
}

fn contains_any(text: &str, subs: &[&str]) -> bool {
    subs.iter().any(|sub| text.contains(sub))
}

/// Upload messages worth an immediate re-send, per step.
fn retryable_messages(step: Step) -> &'static [&'static str] {
    match step {
        Step::InvoiceZip => &[
            "404 NOT FOUND NO SUCH OBJECT: RDFS_PROD/FACTURACION_ELECTRONICA",
            "LA ESTRUCTURA DEL XML NO CUMPLE CON EL ESTANDAR NORMATIVO",
            "CONNECTION TIMED OU",
        ],
        Step::Cuv => &[
            "EN ESTE MOMENTO NO ES POSIBLE VERIFICAR EL CUV",
            "NO CUENTA CON UNA FACTURA ELECTRONICA",
            "OCURRIO UN ERROR EN EL LLAMADO AL SERVICIO",
            "FIREBASE ERROR HTTP=500",
            "EL CUV NO CUMPLE CON EL CONJUNTO DE INFORMACION VALIDADA POR EL MINISTERIO",
        ],
        Step::Rips => &["SEÑOR USUARIO, PARA CONTINUAR"],
        Step::Crc => &[
            "EL SOPORTE QUE DESEA CARGAR NO TIENE FACTURA/CUV CARGADA",
            "ERROR",
        ],
    }
}

/// A failed upload message of `step` that is transient: re-send right away.
pub fn retryable_upload_message(step: Step, msg: &str) -> bool {
    contains_any(&upper(msg), retryable_messages(step))
}

/// A step 2 failure meaning Mutual Ser lost the step 1 invoice: step 1 must be
/// re-sent before retrying step 2.
pub fn requires_step1_reload(detail: &str) -> bool {
    contains_any(
        &upper(detail),
        &[
            "RF-FE-012",
            "RF-FE-016",
            "NO CUENTA CON UNA FACTURA ELECTRONICA",
            "ERROR EN EL LLAMADO AL SERVICIO PARA VALIDAR LA EXISTENCIA DE LA FACTURA",
            "ERROR EN LA LLAMADA AL SERVICIO PARA VALIDAR EL CUV",
        ],
    )
}

/// Mutual Ser rejected the invoice itself (a credit note is required; no step
/// can succeed anymore).
pub fn global_rejection(text: &str) -> bool {
    contains_any(
        &upper(text),
        &[
            "FACTURA ASOCIADA SE ENCUENTRA EN ESTADO RECHAZADA",
            "SE ENCUENTRA EN ESTADO RECHAZADA",
            "STATUSTYPE=3",
        ],
    )
}

/// Mutual Ser rejected the invoice with the `RFFE022` code (the 22
/// business-day limit). Text that only mentions "22 días hábiles" is not an
/// expiry: Mutual Ser may still accept the invoice.
pub fn expired_rejection(text: &str) -> bool {
    upper(text).contains("RFFE022")
}

/// A step 4 failure that requires an adjustment note (the JSON is loaded but
/// Mutual Ser refuses the CRC support).
pub fn crc_adjustment_note(detail: &str) -> bool {
    let t = upper(detail);
    if contains_any(
        &t,
        &[
            "SOPORTEACTIVO=FALSE",
            "DOCUMENTO_SOPORTE=NULL",
            "ISSUPPORTDOCUMENT=FALSE",
            "SOPORTEPENDIENTE=TRUE",
        ],
    ) {
        return false;
    }
    contains_any(
        &t,
        &[
            "NO PERMIT",
            "NO TIENE FACTURA/CUV",
            "FACTURA/CUV CARGADA",
            "NO CUENTA CON FACTURA",
            "RECHAZAD",
            "ERROR ",
        ],
    )
}

/// The step 4 "support already exists" message.
pub fn support_already_associated(text: &str) -> bool {
    upper(text).contains("YA CUENTA CON UN SOPORTE ASOCIADO")
}

/// A previous upload is still being processed by Mutual Ser (do not re-send yet).
pub fn still_processing(detail: &str) -> bool {
    contains_any(
        &detail.to_uppercase(),
        &[
            "EN_PROCESO",
            "EN_PROGRESO",
            "SIGUE PROCESANDO",
            "NO FINALIZO",
            "ENCONTRADO=FALSE",
            "SOPORTEACTIVO=FALSE",
            "DOCUMENTO_SOPORTE=NULL",
        ],
    )
}

static FECHA_RE: LazyLock<Regex> = LazyLock::new(|| {
    // A constant pattern: it always compiles.
    Regex::new(r#""FECHA":"([0-9T:.\-]+Z)""#).unwrap_or_else(|_| unreachable!())
});

/// The "fecha" of an upload response is at least `after` old: the upload never
/// progressed and can be re-sent (step 4).
pub fn upload_lost(response: &str, now: DateTime<Utc>, after: std::time::Duration) -> bool {
    let upper = response.to_uppercase();
    let Some(found) = FECHA_RE.captures(&upper) else {
        return false;
    };
    let Ok(then) = DateTime::parse_from_rfc3339(&found[1]) else {
        return false;
    };
    (now - then.with_timezone(&Utc))
        .to_std()
        .is_ok_and(|age| age >= after)
}

/// Upload messages that already mean success.
pub fn initial_success_message(msg: &str) -> bool {
    contains_any(
        &upper(msg),
        &[
            "EXITOSO",
            "YA SE ENCUENTRA EN ESTADO VALIDADO",
            "YA CUENTA CON UN SOPORTE ASOCIADO",
            "CUV INGRESADO YA SE ENCUENTRA ASOCIADO",
        ],
    )
}

/// The step 2 ministry validation failure, which allows re-sending the TXT
/// without re-sending step 1.
pub fn cuv_not_ministry_compliant(detail: &str) -> bool {
    upper(detail).contains("NO CUMPLE CON EL CONJUNTO DE INFORMACION VALIDADA POR EL MINISTERIO")
}

/// A CUV TXT (the ministry JSON response) carries a valid CUV, or states that
/// the CUV already exists / was validated.
pub fn txt_has_cuv(content: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(content) else {
        return false;
    };
    if find_cuv(&value).is_some_and(|s| valid_cuv(&s)) {
        return true;
    }
    let text = String::from_utf8_lossy(content).to_uppercase();
    text.contains("CUV") && contains_any(&text, &["YA SE ENCUENTRA", "YA EXISTE", "VALIDADO"])
}

/// The valid CUV found in a CUV TXT.
pub fn txt_cuv(content: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(content).ok()?;
    find_cuv(&value).filter(|s| valid_cuv(s))
}

fn is_cuv_key(key: &str) -> bool {
    [
        "CodigoUnicoValidacion",
        "CodigoUnicoValidacionToShow",
        "CUV",
    ]
    .iter()
    .any(|name| name.eq_ignore_ascii_case(key))
}

fn find_cuv(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                if is_cuv_key(key) {
                    let text = json_text(val);
                    if valid_cuv(&text) {
                        return Some(text.trim().to_owned());
                    }
                }
            }
            map.values()
                .find_map(find_cuv)
                .filter(|found| valid_cuv(found))
        }
        Value::Array(items) => items
            .iter()
            .find_map(find_cuv)
            .filter(|found| valid_cuv(found)),
        _ => None,
    }
}

fn json_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn valid_cuv(s: &str) -> bool {
    let s = s.trim();
    let u = s.to_uppercase();
    !s.is_empty() && s != "-" && u != "NULL" && !u.contains("NO APLICA")
}

/// Value validation block codes (legacy names, persisted in
/// `radicacion_estado.last_detail`).
pub const CODE_INVOICE_VALUE_UNAVAILABLE: &str = "VALOR_FACTURA_NO_DISPONIBLE";
pub const CODE_AGREEMENT_VALUE_UNAVAILABLE: &str = "VALOR_CONVENIO_NO_DISPONIBLE";
pub const CODE_VALUE_DIFFERENT: &str = "VALOR_FACTURA_DIFERENTE";
pub const CODE_VALIDATION_UNAVAILABLE: &str = "VALIDACION_VALOR_NO_DISPONIBLE";

/// Compares the invoice XML total with the agreement (convenio) total using
/// exact decimal arithmetic (tolerance 0.01). `None` means step 1 may be sent;
/// otherwise the block code and its detail message.
pub fn check_value(
    xml_total: &str,
    xml_ok: bool,
    agreement_total: &str,
    agreement_ok: bool,
) -> Option<(&'static str, String)> {
    if !xml_ok {
        return Some((
            CODE_INVOICE_VALUE_UNAVAILABLE,
            "No se pudo extraer ni consultar el total XML antes de radicar.".to_owned(),
        ));
    }
    let Some(xml) = parse_decimal(&xml_total.trim().replace(',', "")) else {
        return Some((
            CODE_VALIDATION_UNAVAILABLE,
            format!(
                "No fue posible comprobar los valores antes de radicar: valor XML invalido {xml_total}"
            ),
        ));
    };
    if !agreement_ok {
        return Some((
            CODE_AGREEMENT_VALUE_UNAVAILABLE,
            format!(
                "No existe valor total del informe de convenio para comparar. valor_xml={}",
                decimal_string(xml)
            ),
        ));
    }
    let Some(agreement) = parse_decimal(agreement_total.trim()) else {
        return Some((
            CODE_VALIDATION_UNAVAILABLE,
            format!(
                "No fue posible comprobar los valores antes de radicar: valor convenio invalido {agreement_total}"
            ),
        ));
    };
    let diff = xml - agreement;
    if diff.abs() >= Decimal::new(1, 2) {
        return Some((
            CODE_VALUE_DIFFERENT,
            format!(
                "El total del XML no coincide con el informe de convenio. valor_xml={} valor_convenio={} diferencia={}",
                decimal_string(xml),
                decimal_string(agreement),
                decimal_string(diff)
            ),
        ));
    }
    None
}

fn parse_decimal(text: &str) -> Option<Decimal> {
    text.parse::<Decimal>()
        .ok()
        .or_else(|| Decimal::from_scientific(text).ok())
}

/// Plain notation, at most 6 decimals, trailing zeros trimmed.
fn decimal_string(value: Decimal) -> String {
    value
        .round_dp_with_strategy(6, RoundingStrategy::MidpointAwayFromZero)
        .normalize()
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn retryable_messages_are_per_step() {
        let cases = [
            (
                Step::InvoiceZip,
                "404 Not Found No such object: rdfs_prod/facturacion_electronica/x",
                true,
            ),
            (Step::InvoiceZip, "Connection timed out", true),
            (
                Step::InvoiceZip,
                "La estructura del XML no cumple con el estándar normativo",
                true,
            ),
            (Step::InvoiceZip, "factura duplicada", false),
            (
                Step::Cuv,
                "En este momento no es posible verificar el CUV",
                true,
            ),
            (Step::Cuv, "ERROR → Firebase error HTTP=500", true),
            (
                Step::Cuv,
                "El CUV no cumple con el conjunto de información validada por el ministerio",
                true,
            ),
            (Step::Rips, "Señor usuario, para continuar debe...", true),
            (Step::Rips, "Connection timed out", false),
            (
                Step::Crc,
                "El soporte que desea cargar no tiene factura/cuv cargada",
                true,
            ),
            (Step::Crc, "Error inesperado", true),
            (Step::Crc, "todo bien", false),
        ];
        for (step, msg, want) in cases {
            assert_eq!(retryable_upload_message(step, msg), want, "{step:?} {msg}");
        }
    }

    #[test]
    fn step1_reload_markers() {
        for yes in [
            "codigo=RF-FE-012 mensaje=x",
            "RF-FE-016",
            "La factura no cuenta con una factura electrónica",
            "Error en el llamado al servicio para validar la existencia de la factura",
            "ERROR EN LA LLAMADA AL SERVICIO PARA VALIDAR EL CUV",
        ] {
            assert!(requires_step1_reload(yes), "{yes}");
        }
        for no in ["", "RF-FE-013", "todo ok"] {
            assert!(!requires_step1_reload(no), "{no}");
        }
    }

    #[test]
    fn global_rejection_markers() {
        for yes in [
            "La factura asociada se encuentra en estado RECHAZADA",
            "se encuentra en estado rechazada",
            "filter-xml status=INACTIVE statusType=3",
        ] {
            assert!(global_rejection(yes), "{yes}");
        }
        assert!(!global_rejection("statusType=2"));
    }

    #[test]
    fn expired_rejection_needs_the_rffe022_code() {
        for (text, want) in [
            ("RFFE022 x", true),
            ("rffe022", true),
            ("Rechazada: rFfE022", true),
            ("RFFE022: supera 22 días hábiles", true),
            ("supera 22 días hábiles", false),
            ("22 DIAS HABILES", false),
            ("22 DÍAS HÁBILES", false),
            ("rechazada", false),
            ("", false),
        ] {
            assert_eq!(expired_rejection(text), want, "{text}");
        }
    }

    #[test]
    fn crc_adjustment_note_rules() {
        for (text, want) in [
            ("No permitido asociar", true),
            ("no tiene factura/cuv", true),
            ("RECHAZADO", true),
            ("ERROR al cargar", true),
            ("ERROR", false),
            ("no permitido soporteActivo=false", false),
            ("rechazado documento_soporte=null", false),
            ("rechazado isSupportDocument=false", false),
            ("rechazado soportePendiente=true", false),
            ("ok", false),
        ] {
            assert_eq!(crc_adjustment_note(text), want, "{text}");
        }
    }

    #[test]
    fn still_processing_markers() {
        for yes in [
            "estado=EN_PROCESO",
            "EN_PROGRESO",
            "sigue procesando",
            "no finalizo",
            "encontrado=false",
            "soporteActivo=false",
            "documento_soporte=null",
        ] {
            assert!(still_processing(yes), "{yes}");
        }
        assert!(!still_processing("estado=TERMINADO"));
    }

    #[test]
    fn upload_lost_reads_the_fecha_of_the_response() {
        let now = DateTime::parse_from_rfc3339("2026-09-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let after = Duration::from_secs(90);
        for (response, want) in [
            (r#"{"fecha":"2026-09-25T11:58:00Z"}"#, true),
            (r#"{"fecha":"2026-09-25T11:59:30Z"}"#, false),
            (r#"{"fecha":"2026-09-25T11:58:00.123Z"}"#, true),
            (r#"{"estado":"EN_PROCESO"}"#, false),
            (r#"{"fecha":"garbage-Z"}"#, false),
        ] {
            assert_eq!(upload_lost(response, now, after), want, "{response}");
        }
    }

    #[test]
    fn success_and_ministry_messages() {
        for yes in [
            "Exitoso",
            "La factura ya se encuentra en estado VALIDADO",
            "ya cuenta con un soporte asociado",
            "CUV ingresado ya se encuentra asociado",
        ] {
            assert!(initial_success_message(yes), "{yes}");
        }
        assert!(!initial_success_message("error"));
        assert!(cuv_not_ministry_compliant(
            "El CUV no cumple con el conjunto de información validada por el Ministerio"
        ));
        assert!(!cuv_not_ministry_compliant("ok"));
        assert!(support_already_associated(
            "Ya cuenta con un soporte asociado"
        ));
    }

    #[test]
    fn txt_cuv_detection() {
        for (name, input, want) in [
            (
                "top-level key",
                r#"{"CodigoUnicoValidacion":"abc123"}"#,
                true,
            ),
            (
                "nested case-insensitive",
                r#"{"ResultState":{"resultados":[{"cuv":"x1"}]}}"#,
                true,
            ),
            (
                "to-show key",
                r#"{"codigounicovalidaciontoshow":"x"}"#,
                true,
            ),
            ("dash", r#"{"CUV":"-"}"#, false),
            ("null literal", r#"{"CUV":null}"#, false),
            ("NULL string", r#"{"CUV":"null"}"#, false),
            ("no aplica", r#"{"CUV":"No aplica"}"#, false),
            (
                "text says exists",
                r#"{"mensaje":"El CUV ya se encuentra registrado"}"#,
                true,
            ),
            ("text validado", r#"{"m":"CUV VALIDADO"}"#, true),
            ("not json", "CUV ya existe", false),
            ("empty", "", false),
        ] {
            assert_eq!(txt_has_cuv(input.as_bytes()), want, "{name}");
        }
        assert_eq!(
            txt_cuv(br#"{"CodigoUnicoValidacion":" abc123 "}"#).as_deref(),
            Some("abc123")
        );
        assert_eq!(txt_cuv(br#"{"CUV":"-"}"#), None);
        assert_eq!(
            txt_cuv(br#"{"mensaje":"El CUV ya se encuentra registrado"}"#),
            None
        );
        assert_eq!(txt_cuv(b"not json"), None);
    }

    #[test]
    fn value_check_codes_and_tolerance() {
        let code = |xml: &str, xml_ok, agr: &str, agr_ok| {
            check_value(xml, xml_ok, agr, agr_ok).map(|(code, _)| code)
        };
        assert_eq!(code("1000.00", true, "1000", true), None);
        assert_eq!(code("1000.004", true, "1000", true), None);
        assert_eq!(
            code("1000.01", true, "1000", true),
            Some(CODE_VALUE_DIFFERENT)
        );
        assert_eq!(
            code("999.99", true, "1000", true),
            Some(CODE_VALUE_DIFFERENT)
        );
        assert_eq!(
            code("", false, "1000", true),
            Some(CODE_INVOICE_VALUE_UNAVAILABLE)
        );
        assert_eq!(
            code("1000", true, "", false),
            Some(CODE_AGREEMENT_VALUE_UNAVAILABLE)
        );
        assert_eq!(
            code("abc", true, "1000", true),
            Some(CODE_VALIDATION_UNAVAILABLE)
        );
        assert_eq!(
            code("1000", true, "abc", true),
            Some(CODE_VALIDATION_UNAVAILABLE)
        );
    }

    #[test]
    fn value_check_messages_are_the_stored_contract() {
        let (_, detail) = check_value("1,200.50", true, "1000", true).unwrap();
        assert_eq!(
            detail,
            "El total del XML no coincide con el informe de convenio. valor_xml=1200.5 valor_convenio=1000 diferencia=200.5"
        );
        let (_, detail) = check_value("700", true, "", false).unwrap();
        assert_eq!(
            detail,
            "No existe valor total del informe de convenio para comparar. valor_xml=700"
        );
        let (_, detail) = check_value("", false, "", false).unwrap();
        assert_eq!(
            detail,
            "No se pudo extraer ni consultar el total XML antes de radicar."
        );
        let (_, detail) = check_value("abc", true, "1", true).unwrap();
        assert_eq!(
            detail,
            "No fue posible comprobar los valores antes de radicar: valor XML invalido abc"
        );
    }
}
