//! Affiliate identity, validated rights and the FHIR patient mapping.
//!
//! Port of Go `internal/derechos/derechos.go`. The validator is mandatory: the
//! CSV `causal exento`, `departamento` and `municipio` columns are never a
//! fallback for these fields (except in prepared-format loads, see
//! `prepared.rs`).

use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::Value;

use crate::stores::derechos::Rights as CachedRights;

/// Manual-review reason: the validator does not know the affiliate.
pub const REASON_NOT_FOUND: &str = "Afiliado no encontrado en validador de derechos Mutual Ser";
/// Manual-review reason: technical validator failure.
pub const REASON_UNAVAILABLE: &str = "Validador de derechos no disponible";
/// Prefix of the reason for incomplete data (the missing fields follow).
pub const REASON_INCOMPLETE: &str = "Datos de derechos incompletos: faltan ";
/// Manual-review reason: the invoice has no affiliate document.
pub const REASON_NO_DOCUMENT: &str = "Factura sin tipo o numero de documento del afiliado";

const EXT_HEALTH_MODALITY: &str = "mutualSER/hl7/patient/healthModality";
const EXT_ZONE: &str = "mutualSER/hl7/patient/zone";

/// A patient identified by document (`documentType` / `documentId`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AffiliateQuery {
    pub doc_type: String,
    pub doc_number: String,
}

impl AffiliateQuery {
    /// From raw CSV cells: type upper-cased and trimmed, number trimmed.
    pub fn normalized(doc_type: &str, doc_number: &str) -> Self {
        Self {
            doc_type: doc_type.trim().to_uppercase(),
            doc_number: doc_number.trim().to_owned(),
        }
    }

    /// `"type|number"`, the identity of the once-per-affiliate query.
    pub fn key(&self) -> String {
        format!("{}|{}", self.doc_type, self.doc_number)
    }

    /// False when the type or the number is missing.
    pub fn has_document(&self) -> bool {
        !self.doc_type.is_empty() && !self.doc_number.is_empty()
    }
}

/// The validated user data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rights {
    /// `01` contributivo, `04` subsidiado.
    pub user_type: String,
    /// `U` urbana, `R` rural.
    pub zone: String,
    /// Municipality DANE code.
    pub dane: String,
    /// `M`, `F`, `I`.
    pub sex: String,
    /// `yyyy-MM-dd`, as returned.
    pub birth_date: String,
}

impl Rights {
    /// Labels of the absent or invalid fields (Spanish, used in the manual
    /// review reason). Empty means complete.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.user_type != "01" && self.user_type != "04" {
            missing.push("regimen");
        }
        if self.zone != "U" && self.zone != "R" {
            missing.push("zona");
        }
        if self.dane.trim().is_empty() {
            missing.push("municipio DANE");
        }
        if self.sex.trim().is_empty() {
            missing.push("sexo");
        }
        if self.birth_date.trim().is_empty() {
            missing.push("fecha de nacimiento");
        }
        missing
    }
}

impl From<CachedRights> for Rights {
    fn from(cached: CachedRights) -> Self {
        Self {
            user_type: cached.user_type,
            zone: cached.zone,
            dane: cached.dane,
            sex: cached.sex,
            birth_date: cached.birth_date,
        }
    }
}

impl From<&Rights> for CachedRights {
    fn from(rights: &Rights) -> Self {
        Self {
            user_type: rights.user_type.clone(),
            zone: rights.zone.clone(),
            dane: rights.dane.clone(),
            sex: rights.sex.clone(),
            birth_date: rights.birth_date.clone(),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Bundle {
    entry: Option<Vec<Option<Entry>>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Entry {
    resource: Option<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Patient {
    extension: Option<Vec<Extension>>,
    gender: Option<String>,
    #[serde(rename = "birthDate")]
    birth_date: Option<String>,
    address: Option<Vec<Address>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Extension {
    url: Option<String>,
    #[serde(rename = "valueCoding")]
    value_coding: Option<Coding>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Coding {
    display: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Address {
    city: Option<String>,
    extension: Option<Vec<AddressExtension>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AddressExtension {
    url: Option<String>,
    #[serde(rename = "valueString")]
    value_string: Option<String>,
}

/// Reads a `validateRights` answer (a FHIR Bundle). `Ok(None)` means the
/// bundle has no entry: the affiliate was not found. An undecodable body is an
/// error (a technical failure of the validator).
pub fn parse_response(body: &[u8]) -> Result<Option<Rights>, String> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|err| format!("derechos: invalid validator response: {err}"))?;
    if value.is_null() {
        return Ok(None);
    }
    let bundle: Bundle = serde_json::from_value(value)
        .map_err(|err| format!("derechos: invalid validator response: {err}"))?;
    let resource = bundle
        .entry
        .and_then(|entries| entries.into_iter().next().flatten())
        .and_then(|entry| entry.resource)
        .filter(|resource| !resource.is_null());
    let Some(resource) = resource else {
        return Ok(None);
    };
    let patient: Patient = match resource {
        Value::String(text) => serde_json::from_str(&text),
        other => serde_json::from_value(other),
    }
    .map_err(|err| format!("derechos: invalid patient resource: {err}"))?;
    Ok(Some(rights_of(&patient)))
}

fn rights_of(patient: &Patient) -> Rights {
    let mut rights = Rights::default();
    let modality = patient
        .extension
        .iter()
        .flatten()
        .find(|ext| ext.url.as_deref() == Some(EXT_HEALTH_MODALITY));
    if let Some(ext) = modality {
        let display = ext
            .value_coding
            .as_ref()
            .and_then(|coding| coding.display.as_deref())
            .unwrap_or_default();
        rights.user_type = if display.to_uppercase().contains("SUBSIDIADO") {
            "04".into()
        } else {
            "01".into()
        };
    }
    if let Some(address) = patient.address.iter().flatten().next() {
        let zone = address
            .extension
            .iter()
            .flatten()
            .filter(|ext| ext.url.as_deref() == Some(EXT_ZONE))
            .find_map(|ext| ext.value_string.as_deref());
        if let Some(zone) = zone {
            rights.zone = if zone.trim().is_empty() || zone.contains("Urb") {
                "U".into()
            } else {
                "R".into()
            };
        }
        rights.dane = dane_of(address.city.as_deref().unwrap_or_default());
    }
    rights.sex = match patient.gender.as_deref() {
        Some("male") => "M",
        Some("female") => "F",
        // Legacy: any other or empty gender is "indeterminado".
        _ => "I",
    }
    .into();
    rights.birth_date = patient
        .birth_date
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_owned();
    rights
}

/// DANE code of `address[0].city`: `code=`/`display=` removed, first `|` part.
fn dane_of(city: &str) -> String {
    let cleaned = city.replace("code=", "").replace("display=", "");
    cleaned
        .split('|')
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// Document type retried when the validator rejects the request (400-410):
/// `TI -> CC`, `RC -> TI` (so `RC -> TI -> CC` over three hops).
pub fn alternate_doc_type(doc_type: &str) -> Option<&'static str> {
    match doc_type.trim().to_uppercase().as_str() {
        "TI" => Some("CC"),
        "RC" => Some("TI"),
        _ => None,
    }
}

/// Fields where the convenio CSV disagrees with the validator. Values that
/// cannot be interpreted are not compared; the validator always wins and the
/// result is only reported (by field name).
pub fn cross_check(rights: &Rights, csv_gender: &str, csv_birth_date: &str) -> Vec<&'static str> {
    let mut differences = Vec::new();
    let gender = csv_gender.trim().to_uppercase();
    if (gender == "M" || gender == "F") && !rights.sex.is_empty() && gender != rights.sex {
        differences.push("sexo");
    }
    if let (Some(csv), Some(validated)) = (
        parse_cross_check_date(csv_birth_date),
        parse_cross_check_date(&rights.birth_date),
    ) && csv != validated
    {
        differences.push("fecha de nacimiento");
    }
    differences
}

/// First 10 bytes as `yyyy-MM-dd` or `dd/MM/yyyy` (Go `parseDate`).
fn parse_cross_check_date(text: &str) -> Option<NaiveDate> {
    let head = text.trim().get(..10)?;
    NaiveDate::parse_from_str(head, "%Y-%m-%d")
        .ok()
        .filter(|_| is_iso_shape(head))
        .or_else(|| {
            NaiveDate::parse_from_str(head, "%d/%m/%Y")
                .ok()
                .filter(|_| is_dmy_shape(head))
        })
}

fn digits_at(text: &str, range: std::ops::Range<usize>) -> bool {
    text.get(range)
        .is_some_and(|part| part.bytes().all(|b| b.is_ascii_digit()))
}

fn is_iso_shape(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && digits_at(text, 0..4)
        && digits_at(text, 5..7)
        && digits_at(text, 8..10)
}

fn is_dmy_shape(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[2] == b'/'
        && bytes[5] == b'/'
        && digits_at(text, 0..2)
        && digits_at(text, 3..5)
        && digits_at(text, 6..10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patient_bundle(modality: &str, zone: Option<&str>, city: &str, gender: &str) -> String {
        let zone_ext = zone.map_or(String::new(), |z| {
            format!(r#""extension":[{{"url":"{EXT_ZONE}","valueString":"{z}"}}],"#)
        });
        format!(
            r#"{{"resourceType":"Bundle","entry":[{{"resource":{{"extension":[{{"url":"{EXT_HEALTH_MODALITY}","valueCoding":{{"display":"{modality}"}}}}],"gender":"{gender}","birthDate":" 1990-01-31 ","address":[{{{zone_ext}"city":"{city}"}}]}}}}]}}"#
        )
    }

    #[test]
    fn missing_lists_the_spanish_labels_in_order() {
        assert_eq!(
            Rights::default().missing(),
            [
                "regimen",
                "zona",
                "municipio DANE",
                "sexo",
                "fecha de nacimiento"
            ]
        );
        let complete = Rights {
            user_type: "04".into(),
            zone: "R".into(),
            dane: "13001".into(),
            sex: "I".into(),
            birth_date: "2000-01-01".into(),
        };
        assert!(complete.missing().is_empty());
        let odd = Rights {
            user_type: "02".into(),
            zone: "u".into(),
            dane: "  ".into(),
            ..complete
        };
        assert_eq!(odd.missing(), ["regimen", "zona", "municipio DANE"]);
    }

    #[test]
    fn maps_subsidiado_urban_and_the_dane_prefix() {
        let body = patient_bundle(
            "Régimen SUBSIDIADO",
            Some("Urbana"),
            "13001|CARTAGENA",
            "male",
        );
        let rights = parse_response(body.as_bytes())
            .expect("parse")
            .expect("found");
        assert_eq!(
            rights,
            Rights {
                user_type: "04".into(),
                zone: "U".into(),
                dane: "13001".into(),
                sex: "M".into(),
                birth_date: "1990-01-31".into(),
            }
        );
    }

    #[test]
    fn maps_contributivo_rural_and_strips_code_and_display_markers() {
        let body = patient_bundle(
            "Contributivo",
            Some("Rural"),
            "code=47001|display=SANTA MARTA",
            "female",
        );
        let rights = parse_response(body.as_bytes())
            .expect("parse")
            .expect("found");
        assert_eq!(rights.user_type, "01");
        assert_eq!(rights.zone, "R");
        assert_eq!(rights.dane, "47001");
        assert_eq!(rights.sex, "F");
    }

    #[test]
    fn an_empty_zone_is_urban_and_a_missing_zone_extension_stays_missing() {
        let blank = patient_bundle("X", Some("  "), "1|A", "other");
        let rights = parse_response(blank.as_bytes())
            .expect("parse")
            .expect("found");
        assert_eq!((rights.zone.as_str(), rights.sex.as_str()), ("U", "I"));
        let absent = patient_bundle("X", None, "1|A", "");
        let rights = parse_response(absent.as_bytes())
            .expect("parse")
            .expect("found");
        assert_eq!((rights.zone.as_str(), rights.sex.as_str()), ("", "I"));
        assert_eq!(rights.missing(), ["zona"]);
    }

    #[test]
    fn a_missing_modality_extension_leaves_the_regimen_empty() {
        let body = r#"{"entry":[{"resource":{"gender":"male","birthDate":"2001-02-03"}}]}"#;
        let rights = parse_response(body.as_bytes())
            .expect("parse")
            .expect("found");
        assert_eq!(rights.user_type, "");
        assert_eq!(rights.dane, "");
    }

    #[test]
    fn the_resource_may_be_a_json_string() {
        let inner = r#"{"gender":"female","birthDate":"2010-05-06"}"#;
        let body = serde_json::json!({"entry": [{"resource": inner}]}).to_string();
        let rights = parse_response(body.as_bytes())
            .expect("parse")
            .expect("found");
        assert_eq!(rights.sex, "F");
        assert_eq!(rights.birth_date, "2010-05-06");
    }

    #[test]
    fn empty_bundles_and_null_resources_are_not_found() {
        for body in [
            r#"{}"#,
            r#"{"entry":[]}"#,
            r#"{"entry":null}"#,
            r#"{"entry":[{"resource":null}]}"#,
            r#"{"entry":[{}]}"#,
            "null",
        ] {
            assert_eq!(parse_response(body.as_bytes()), Ok(None), "{body}");
        }
    }

    #[test]
    fn undecodable_answers_are_errors() {
        assert!(parse_response(b"<html>").is_err());
        assert!(parse_response(b"[1]").is_err());
        assert!(parse_response(br#"{"entry":[{"resource":"not json"}]}"#).is_err());
        assert!(parse_response(br#"{"entry":[{"resource":7}]}"#).is_err());
    }

    #[test]
    fn alternate_document_types_follow_the_java_recursion() {
        assert_eq!(alternate_doc_type("TI"), Some("CC"));
        assert_eq!(alternate_doc_type(" rc "), Some("TI"));
        assert_eq!(alternate_doc_type("CC"), None);
        assert_eq!(alternate_doc_type(""), None);
    }

    #[test]
    fn affiliate_identity_is_normalized() {
        let a = AffiliateQuery::normalized(" ti ", " 123 ");
        assert_eq!(a.key(), "TI|123");
        assert!(a.has_document());
        assert!(!AffiliateQuery::normalized("CC", "  ").has_document());
    }

    #[test]
    fn cross_check_reports_field_names_only() {
        let rights = Rights {
            sex: "F".into(),
            birth_date: "1990-01-31".into(),
            ..Rights::default()
        };
        assert_eq!(cross_check(&rights, "m", "31/01/1990"), ["sexo"]);
        assert_eq!(
            cross_check(&rights, "F", "1990-02-01 00:00"),
            ["fecha de nacimiento"]
        );
        assert!(cross_check(&rights, "F", "31/01/1990").is_empty());
        // Uninterpretable CSV values are not compared.
        assert!(cross_check(&rights, "X", "garbage").is_empty());
        assert!(cross_check(&rights, "", "").is_empty());
        assert!(cross_check(&rights, "I", "31-01-1990").is_empty());
    }
}
