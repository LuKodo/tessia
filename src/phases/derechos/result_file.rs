//! The per-invoice result file `derechos/<invoice>.json` (artifact rel path),
//! read by `rips generar`.
//!
//! The format is byte-compatible with Go's `json.MarshalIndent(rec, "", "  ")`
//! plus a trailing newline (see `docs/derechos.md`): fixed member order, two
//! space indent, `omitempty` members dropped, Go's HTML-safe string escaping.
//! It holds personal data.
//!
//! ```json
//! {
//!   "version": 1,
//!   "validado_en": "2026-10-22T15:04:05.123456Z",
//!   "estado": "DONE",
//!   "afiliado": {
//!     "tipo_documento": "CC",
//!     "numero_documento": "123",
//!     "derechos": {
//!       "tipo_usuario": "04",
//!       "zona": "U",
//!       "municipio_dane": "13001",
//!       "sexo": "F",
//!       "fecha_nacimiento": "1990-01-31"
//!     },
//!     "origen": "validador"
//!   }
//! }
//! ```
//!
//! Members: `estado`; `motivo` (omitted when empty); `afiliado` (only for
//! `DONE`) with `tipo_documento_consultado` (omitted unless the validator
//! answered for another type); `diferencias_csv` (omitted when empty).

use std::fmt::Write as _;

use anyhow::{Context, Result};
use chrono::{DateTime, Timelike, Utc};
use serde::Deserialize;

use super::service::{AffiliateRights, InvoiceResult};
use crate::domain::normalize_invoice;
use crate::storage::Artifacts;

/// Folder (and artifact prefix) of the result files.
pub const RESULT_DIR: &str = "derechos";

/// Artifact relative path of the invoice's file: `derechos/<INVOICE>.json`.
pub fn rel_path(invoice: &str) -> String {
    format!("{RESULT_DIR}/{}.json", normalize_invoice(invoice))
}

/// The file content for `result`, validated at `now`.
pub fn render(result: &InvoiceResult, now: DateTime<Utc>) -> Vec<u8> {
    let mut out = String::from("{\n  \"version\": 1,\n");
    let _ = writeln!(out, "  \"validado_en\": {},", quote(&go_time(now)));
    let _ = write!(out, "  \"estado\": {}", quote(result.state.as_str()));
    if !result.reason.is_empty() {
        let _ = write!(out, ",\n  \"motivo\": {}", quote(&result.reason));
    }
    if let Some(affiliate) = &result.affiliate {
        out.push_str(",\n  \"afiliado\": ");
        write_affiliate(&mut out, affiliate);
    }
    if !result.mismatches.is_empty() {
        out.push_str(",\n  \"diferencias_csv\": [\n");
        let items: Vec<String> = result
            .mismatches
            .iter()
            .map(|m| format!("    {}", quote(m)))
            .collect();
        out.push_str(&items.join(",\n"));
        out.push_str("\n  ]");
    }
    out.push_str("\n}\n");
    out.into_bytes()
}

fn write_affiliate(out: &mut String, affiliate: &AffiliateRights) {
    let rights = &affiliate.rights;
    let _ = write!(
        out,
        "{{\n    \"tipo_documento\": {},\n    \"numero_documento\": {},\n    \"derechos\": {{\n      \"tipo_usuario\": {},\n      \"zona\": {},\n      \"municipio_dane\": {},\n      \"sexo\": {},\n      \"fecha_nacimiento\": {}\n    }},\n    \"origen\": {}",
        quote(&affiliate.affiliate.doc_type),
        quote(&affiliate.affiliate.doc_number),
        quote(&rights.user_type),
        quote(&rights.zone),
        quote(&rights.dane),
        quote(&rights.sex),
        quote(&rights.birth_date),
        quote(affiliate.source),
    );
    if let Some(queried) = &affiliate.queried_doc_type {
        let _ = write!(
            out,
            ",\n    \"tipo_documento_consultado\": {}",
            quote(queried)
        );
    }
    out.push_str("\n  }");
}

/// Go `time.Time` JSON: RFC 3339 with the fractional seconds trimmed of
/// trailing zeros, UTC as `Z`.
fn go_time(at: DateTime<Utc>) -> String {
    let mut text = at.format("%Y-%m-%dT%H:%M:%S").to_string();
    let fraction = format!("{:09}", at.nanosecond() % 1_000_000_000);
    let fraction = fraction.trim_end_matches('0');
    if !fraction.is_empty() {
        text.push('.');
        text.push_str(fraction);
    }
    text.push('Z');
    text
}

/// Go `encoding/json` string: `"` `\` and control characters escaped, and
/// `<`, `>`, `&`, U+2028, U+2029 as `\u00XX`/`\u202X` (HTML-safe default).
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || matches!(c, '<' | '>' | '&' | '\u{2028}' | '\u{2029}') => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Publishes the invoice's file as the artifact `derechos/<INVOICE>.json`.
pub fn publish(
    artifacts: &Artifacts,
    cargue_id: &str,
    result: &InvoiceResult,
    now: DateTime<Utc>,
) -> Result<()> {
    artifacts
        .publish_bytes(cargue_id, &rel_path(&result.invoice), &render(result, now))
        .with_context(|| {
            format!(
                "no se pudo publicar el resultado de derechos de {}",
                result.invoice
            )
        })?;
    Ok(())
}

/// A result file as the readers (`rips generar`) see it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StoredResult {
    pub version: u32,
    pub validado_en: String,
    pub estado: String,
    #[serde(default)]
    pub motivo: String,
    pub afiliado: Option<StoredAffiliate>,
    #[serde(default)]
    pub diferencias_csv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StoredAffiliate {
    pub tipo_documento: String,
    pub numero_documento: String,
    pub derechos: StoredRights,
    pub origen: String,
    #[serde(default)]
    pub tipo_documento_consultado: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StoredRights {
    pub tipo_usuario: String,
    pub zona: String,
    pub municipio_dane: String,
    pub sexo: String,
    pub fecha_nacimiento: String,
}

/// Decodes a result file.
pub fn parse(bytes: &[u8]) -> Result<StoredResult> {
    serde_json::from_slice(bytes).context("archivo de resultado de derechos inválido")
}

/// The invoice's published result, `None` when it has none.
pub fn read(artifacts: &Artifacts, cargue_id: &str, invoice: &str) -> Result<Option<StoredResult>> {
    let rel = rel_path(invoice);
    if !artifacts.is_published(cargue_id, &rel)? {
        return Ok(None);
    }
    parse(&artifacts.fetch_bytes(cargue_id, &rel)?).map(Some)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::domain::PhaseState;
    use crate::phases::derechos::rights::{AffiliateQuery, Rights};

    fn done(source: &'static str, queried: Option<&str>, mismatches: &[&str]) -> InvoiceResult {
        let mut result = InvoiceResult {
            invoice: "FEV1".into(),
            state: PhaseState::Done,
            reason: String::new(),
            affiliate: Some(AffiliateRights {
                affiliate: AffiliateQuery::normalized("TI", "123"),
                rights: Rights {
                    user_type: "04".into(),
                    zone: "U".into(),
                    dane: "13001".into(),
                    sex: "F".into(),
                    birth_date: "1990-01-31".into(),
                },
                source,
                queried_doc_type: queried.map(str::to_owned),
            }),
            mismatches: Vec::new(),
        };
        result.mismatches = mismatches.iter().map(|m| (*m).to_owned()).collect();
        result
    }

    #[test]
    fn rel_path_normalizes_the_invoice() {
        assert_eq!(rel_path(" fev-1 "), "derechos/FEV1.json");
    }

    #[test]
    fn a_done_file_matches_go_marshal_indent_byte_for_byte() {
        let at = Utc
            .with_ymd_and_hms(2026, 10, 22, 15, 4, 5)
            .single()
            .expect("date");
        let at = at.with_nanosecond(123_456_000).expect("nanos");
        let text = String::from_utf8(render(
            &done("validador", Some("CC"), &["sexo", "fecha de nacimiento"]),
            at,
        ))
        .expect("utf8");
        let want = r#"{
  "version": 1,
  "validado_en": "2026-10-22T15:04:05.123456Z",
  "estado": "DONE",
  "afiliado": {
    "tipo_documento": "TI",
    "numero_documento": "123",
    "derechos": {
      "tipo_usuario": "04",
      "zona": "U",
      "municipio_dane": "13001",
      "sexo": "F",
      "fecha_nacimiento": "1990-01-31"
    },
    "origen": "validador",
    "tipo_documento_consultado": "CC"
  },
  "diferencias_csv": [
    "sexo",
    "fecha de nacimiento"
  ]
}
"#;
        assert_eq!(text, want);
    }

    #[test]
    fn a_manual_review_file_has_only_state_and_reason() {
        let at = Utc
            .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
            .single()
            .expect("date");
        let result = InvoiceResult {
            invoice: "FEV2".into(),
            state: PhaseState::RevisionManual,
            reason: "Datos de derechos incompletos: faltan sexo & <zona>".into(),
            affiliate: None,
            mismatches: Vec::new(),
        };
        let text = String::from_utf8(render(&result, at)).expect("utf8");
        assert_eq!(
            text,
            "{\n  \"version\": 1,\n  \"validado_en\": \"2026-01-02T03:04:05Z\",\n  \"estado\": \"REVISION_MANUAL\",\n  \"motivo\": \"Datos de derechos incompletos: faltan sexo \\u0026 \\u003czona\\u003e\"\n}\n"
        );
    }

    #[test]
    fn time_keeps_go_fraction_trimming() {
        let base = Utc
            .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
            .single()
            .expect("date");
        assert_eq!(
            go_time(base.with_nanosecond(500_000_000).expect("n")),
            "2026-01-02T03:04:05.5Z"
        );
        assert_eq!(
            go_time(base.with_nanosecond(1).expect("n")),
            "2026-01-02T03:04:05.000000001Z"
        );
    }

    #[test]
    fn quoting_follows_go_escaping() {
        assert_eq!(quote("a\"b\\c\n\u{1}é"), "\"a\\\"b\\\\c\\n\\u0001é\"");
        assert_eq!(quote("\u{2028}"), "\"\\u2028\"");
    }

    #[test]
    fn rendered_files_parse_back() {
        let at = Utc
            .with_ymd_and_hms(2026, 10, 22, 15, 4, 5)
            .single()
            .expect("date");
        let stored = parse(&render(&done("cache", None, &[]), at)).expect("parse");
        assert_eq!(stored.estado, "DONE");
        assert_eq!(stored.version, 1);
        let affiliate = stored.afiliado.expect("affiliate");
        assert_eq!(affiliate.origen, "cache");
        assert_eq!(affiliate.tipo_documento_consultado, None);
        assert_eq!(affiliate.derechos.municipio_dane, "13001");
        assert!(stored.diferencias_csv.is_empty());
    }
}
