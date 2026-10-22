//! Reader of the validated rights of an invoice: the artifact
//! `derechos/<F>.json` written by `derechos validar` (format documented in
//! the Go `docs/derechos.md`, "Outputs"). Go source: `derechos.ReadInvoices`.
//!
//! Example of the file (only the fields RIPS needs are read; others such as
//! `version`, `validado_en`, `diferencias_csv` are ignored):
//!
//! ```json
//! {
//!   "estado": "DONE",
//!   "motivo": "",
//!   "afiliado": {
//!     "tipo_documento": "CC",
//!     "numero_documento": "900000001",
//!     "derechos": {
//!       "tipo_usuario": "01", "zona": "U", "municipio_dane": "13001",
//!       "sexo": "F", "fecha_nacimiento": "1990-01-02"
//!     },
//!     "origen": "validador"
//!   }
//! }
//! ```

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::domain::normalize_invoice;
use crate::storage::{Artifacts, layout};

/// The validated user data (Go `derechos.Rights`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Rights {
    /// `tipo_usuario`: `01` contributivo, `04` subsidiado.
    #[serde(rename = "tipo_usuario", default)]
    pub user_type: String,
    /// `zona`: `U` urbana, `R` rural.
    #[serde(rename = "zona", default)]
    pub zone: String,
    /// `municipio_dane`: municipality code.
    #[serde(rename = "municipio_dane", default)]
    pub dane: String,
    /// `sexo`: `M`, `F`, `I`.
    #[serde(rename = "sexo", default)]
    pub sex: String,
    /// `fecha_nacimiento`: as returned (`yyyy-MM-dd`).
    #[serde(rename = "fecha_nacimiento", default)]
    pub birth_date: String,
}

/// The affiliate of the invoice with its rights (Go `derechos.AffiliateRights`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct AffiliateRights {
    /// `tipo_documento`.
    #[serde(rename = "tipo_documento", default)]
    pub doc_type: String,
    /// `numero_documento`.
    #[serde(rename = "numero_documento", default)]
    pub doc_number: String,
    /// `derechos`.
    #[serde(rename = "derechos", default)]
    pub rights: Rights,
}

impl AffiliateRights {
    /// `"type|number"` (Go `Affiliate.Key`), compared untrimmed.
    pub fn key(&self) -> String {
        format!("{}|{}", self.doc_type, self.doc_number)
    }
}

/// The per-invoice result of `derechos validar` (Go `derechos.Outcome`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct DerechosResult {
    /// `estado`: `DONE`, `REVISION_MANUAL`, ...
    #[serde(rename = "estado", default)]
    pub status: String,
    /// `motivo`.
    #[serde(rename = "motivo", default)]
    pub reason: String,
    /// `afiliado`: set when the status is `DONE`.
    #[serde(rename = "afiliado", default)]
    pub affiliate: Option<AffiliateRights>,
}

/// Artifact path of the result of `invoice` (`derechos/<F>.json`).
pub fn artifact_path(invoice: &str) -> String {
    format!("{}/{}.json", layout::DERECHOS, normalize_invoice(invoice))
}

/// Parses the content of `derechos/<F>.json`.
pub fn parse(bytes: &[u8]) -> Result<DerechosResult, serde_json::Error> {
    serde_json::from_slice(bytes)
}

/// Reads the result of `invoice` from the cargue's artifacts: `None` when the
/// invoice has none (`derechos validar` has not run for it).
pub fn read(
    artifacts: &Artifacts,
    cargue_id: &str,
    invoice: &str,
) -> Result<Option<DerechosResult>> {
    let path = artifact_path(invoice);
    if !artifacts.is_published(cargue_id, &path)? {
        return Ok(None);
    }
    let bytes = artifacts.fetch_bytes(cargue_id, &path)?;
    let result = parse(&bytes).with_context(|| format!("derechos: decode {path}"))?;
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_file_ignoring_extra_fields() {
        let raw = br#"{"version":1,"validado_en":"2026-10-22T10:00:00Z","estado":"DONE",
            "afiliado":{"tipo_documento":"CC","numero_documento":"900000001",
            "derechos":{"tipo_usuario":"01","zona":"U","municipio_dane":"13001","sexo":"F",
            "fecha_nacimiento":"1990-01-02"},"origen":"validador"},"diferencias_csv":["x"]}"#;
        let result = parse(raw).expect("parse");
        assert_eq!(result.status, "DONE");
        let affiliate = result.affiliate.expect("affiliate");
        assert_eq!(affiliate.key(), "CC|900000001");
        assert_eq!(affiliate.rights.dane, "13001");
        assert_eq!(affiliate.rights.birth_date, "1990-01-02");
    }

    #[test]
    fn a_manual_review_file_has_a_reason_and_no_affiliate() {
        let raw = br#"{"estado":"REVISION_MANUAL","motivo":"Validador de derechos no disponible"}"#;
        let result = parse(raw).expect("parse");
        assert_eq!(result.status, "REVISION_MANUAL");
        assert_eq!(result.reason, "Validador de derechos no disponible");
        assert!(result.affiliate.is_none());
        assert!(parse(b"not json").is_err());
    }

    #[test]
    fn the_artifact_path_normalizes_the_invoice() {
        assert_eq!(artifact_path(" fvs-1 "), "derechos/FVS1.json");
    }
}
