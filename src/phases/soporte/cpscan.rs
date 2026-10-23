//! Reader of the CPScan sidecar that `thefactory descargar` leaves per invoice.
//!
//! Format (documented in the Go `docs/thefactory.md`, owned by the `thefactory`
//! writer): `CPSCAN/<F>.cpscan.json` holds the outcome of one completed lookup
//!
//! ```json
//! {"version": 1, "ssc": "123", "documento": true, "origen": "APP",
//!  "adicional": "", "estado": "...", "archivo": "formula-123.jpg",
//!  "ext": "jpg", "consultado": "2026-10-21T10:00:00Z"}
//! ```
//!
//! and `CPSCAN/<F>.support.<ext>` the downloaded file. An empty `archivo`
//! means CPScan answered and has no usable support. Both are bucket artifacts
//! of the cargue; this module only reads them (`soporte generar` never calls
//! CPScan).

use anyhow::Result;
use serde::Deserialize;

use crate::context::AppContext;

/// The only sidecar version the build relies on.
pub const SIDECAR_VERSION: i64 = 1;

/// Kinds of payload (`Stored::ext`).
pub const EXT_PDF: &str = "pdf";
const KNOWN_EXTS: [&str; 6] = ["pdf", "jpg", "png", "gif", "bmp", "tif"];

/// Smallest PDF that counts as a scanned support (Go `archivoValido(pdf, 100)`).
pub const MIN_PC_SUPPORT: usize = 100;

/// The sidecar of one completed CPScan lookup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Stored {
    pub version: i64,
    pub ssc: String,
    /// The API returned a document for the SSC.
    #[serde(rename = "documento")]
    pub found: bool,
    #[serde(rename = "origen")]
    pub origin: String,
    #[serde(rename = "adicional")]
    pub additional: String,
    #[serde(rename = "estado")]
    pub status: String,
    /// Bucket file name or URL that was downloaded (`""` = nothing).
    #[serde(rename = "archivo")]
    pub file: String,
    /// Kind of the payload: pdf, jpg, png, gif, bmp or tif.
    pub ext: String,
    #[serde(rename = "consultado")]
    pub fetched_at: String,
}

/// The CPScan document of the lookup (`GET /documentos/{ssc}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub origin: String,
    pub additional: String,
}

impl Document {
    /// Go `normalizarOrigen`: empty or `NULL` is `NULL`, else upper case.
    pub fn normalized_origin(&self) -> String {
        let origin = self.origin.trim().to_uppercase();
        if origin.is_empty() {
            "NULL".into()
        } else {
            origin
        }
    }
}

impl Stored {
    /// The document, or `None` when the API had none.
    pub fn document(&self) -> Option<Document> {
        self.found.then(|| Document {
            origin: self.origin.clone(),
            additional: self.additional.clone(),
        })
    }

    /// A support file was downloaded.
    pub fn has_file(&self) -> bool {
        !self.file.is_empty()
    }

    /// The file is the one the document pointed at (not a name derived from the origin).
    pub fn from_additional(&self) -> bool {
        self.has_file() && self.found && self.file == self.additional.trim()
    }

    /// Parses a sidecar and rejects what the build cannot rely on.
    pub fn parse(bytes: &[u8]) -> Result<Stored> {
        let stored: Stored =
            serde_json::from_slice(bytes).map_err(|e| anyhow::anyhow!("sidecar invalido: {e}"))?;
        if stored.version != SIDECAR_VERSION {
            anyhow::bail!("sidecar invalido: version {} no soportada", stored.version);
        }
        if stored.has_file() && !KNOWN_EXTS.contains(&stored.ext.as_str()) {
            anyhow::bail!("sidecar invalido: ext {:?}", stored.ext);
        }
        Ok(stored)
    }
}

/// `CPSCAN/<F>.cpscan.json`.
pub fn sidecar_rel(invoice: &str) -> String {
    format!("CPSCAN/{invoice}.cpscan.json")
}

/// `CPSCAN/<F>.support.<ext>`.
pub fn payload_rel(invoice: &str, ext: &str) -> String {
    format!("CPSCAN/{invoice}.support.{ext}")
}

/// A lookup that was loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub stored: Stored,
    pub payload: Vec<u8>,
}

/// Why there is no usable lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Missing {
    /// No sidecar: `thefactory descargar` never completed the lookup.
    NotFetched,
    /// A sidecar exists but it or its payload cannot be used.
    Problem(String),
}

/// Loads the sidecar and its payload of `invoice` from the cargue's artifacts
/// (Go `cpscan.Load`). A sidecar that names a file requires that file to be
/// there and not empty.
pub fn load(
    ctx: &AppContext,
    cargue_id: &str,
    invoice: &str,
) -> Result<std::result::Result<Loaded, Missing>> {
    let artifacts = ctx.artifacts();
    let sidecar = sidecar_rel(invoice);
    if ctx.artefactos().get(cargue_id, &sidecar)?.is_none() {
        return Ok(Err(Missing::NotFetched));
    }
    let raw = match artifacts.fetch_bytes(cargue_id, &sidecar) {
        Ok(raw) => raw,
        Err(err) => {
            return Ok(Err(Missing::Problem(format!(
                "cpscan: no se pudo leer el sidecar: {err:#}"
            ))));
        }
    };
    let stored = match Stored::parse(&raw) {
        Ok(stored) => stored,
        Err(err) => return Ok(Err(Missing::Problem(format!("cpscan: {sidecar}: {err:#}")))),
    };
    if !stored.has_file() {
        return Ok(Ok(Loaded {
            stored,
            payload: Vec::new(),
        }));
    }
    let rel = payload_rel(invoice, &stored.ext);
    let payload = match artifacts.fetch_bytes(cargue_id, &rel) {
        Ok(payload) => payload,
        Err(err) => {
            return Ok(Err(Missing::Problem(format!(
                "cpscan: el sidecar nombra {} pero no se puede leer: {err:#}",
                stored.file
            ))));
        }
    };
    if payload.is_empty() {
        return Ok(Err(Missing::Problem(format!("cpscan: {rel} esta vacio"))));
    }
    Ok(Ok(Loaded { stored, payload }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{"version":1,"ssc":"10","documento":true,"origen":"APP","adicional":"","estado":"OK","archivo":"formula-10.jpg","ext":"jpg","consultado":"2026-10-21T10:00:00Z"}"#;

    #[test]
    fn the_documented_sidecar_parses() {
        let stored = Stored::parse(FULL.as_bytes()).expect("parse");
        assert!(stored.found && stored.has_file());
        assert_eq!(
            (stored.origin.as_str(), stored.ext.as_str()),
            ("APP", "jpg")
        );
        assert_eq!(
            stored.document().map(|d| d.normalized_origin()),
            Some("APP".into())
        );
    }

    #[test]
    fn nothing_found_is_a_valid_sidecar_without_file() {
        let stored = Stored::parse(br#"{"version":1,"ssc":"10","documento":false,"archivo":""}"#)
            .expect("parse");
        assert!(!stored.has_file() && stored.document().is_none());
    }

    #[test]
    fn unusable_sidecars_are_rejected() {
        assert!(Stored::parse(b"{").is_err());
        let err = Stored::parse(br#"{"version":2}"#).expect_err("version");
        assert!(err.to_string().contains("version 2 no soportada"));
        let err = Stored::parse(br#"{"version":1,"archivo":"x","ext":"exe"}"#).expect_err("ext");
        assert!(err.to_string().contains("ext"));
    }

    #[test]
    fn empty_and_null_origins_normalize_to_null() {
        let doc = |origin: &str| Document {
            origin: origin.into(),
            additional: String::new(),
        };
        assert_eq!(doc("").normalized_origin(), "NULL");
        assert_eq!(doc(" pc ").normalized_origin(), "PC");
    }

    #[test]
    fn adicional_files_are_recognized_by_their_reference() {
        let mut stored = Stored::parse(FULL.as_bytes()).expect("parse");
        assert!(!stored.from_additional());
        stored.additional = " formula-10.jpg ".into();
        assert!(stored.from_additional());
    }

    #[test]
    fn artifact_paths_are_flat_cpscan_files() {
        assert_eq!(sidecar_rel("FEV1"), "CPSCAN/FEV1.cpscan.json");
        assert_eq!(payload_rel("FEV1", "pdf"), "CPSCAN/FEV1.support.pdf");
    }
}
