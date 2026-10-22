//! Data exchanged between the Mutual Ser client, the rules and the engine
//! (Go `internal/radicacion/ports.go`).

use std::path::PathBuf;

/// A local file to upload for one invoice step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadFile {
    pub invoice: String,
    /// Where the bytes are on this machine (inside the per-cargue workdir).
    pub path: PathBuf,
    /// Original file name registered in Mutual Ser.
    pub name: String,
    pub size: u64,
}

/// A validation message attached to an uploaded file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    pub kind: String,
    pub code: String,
    pub description: String,
}

/// One file of an upload (`findLoad`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadFile {
    pub id: String,
    pub name: String,
    pub status: String,
    pub messages: Vec<Message>,
    /// `errors[].code` plus the codes of the messages of type ERROR.
    pub error_codes: Vec<String>,
}

/// The `findLoad` view of an upload (`id_cargue`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Load {
    /// Upload status (`EN_PROCESO`, `TERMINADO`, ...).
    pub status: String,
    pub files: Vec<LoadFile>,
    /// Compact response body, kept for diagnostics.
    pub raw: String,
}

/// A `filter-xml` row (electronic invoice + CUV association).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XmlDocument {
    pub bill_id: String,
    pub status: String,
    pub status_type: String,
    pub id: String,
    pub upload_date: String,
    pub validation_date: String,
    pub error_codes: Vec<String>,
}

/// The CRC support associated to an invoice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SupportDocument {
    pub id: String,
    pub file_name: String,
    pub status: String,
    /// Compact JSON object (legacy detail `documento_soporte={...}`).
    pub raw: String,
}

/// A `core/filtersAll` row (RIPS JSON and CRC support).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltersAllItem {
    pub invoice_number: String,
    pub status: String,
    pub event_id: String,
    pub file_id: String,
    pub id_cargue: String,
    pub file_name: String,
    pub returned: bool,
    pub services_valid: bool,
    pub services: String,
    pub is_support_document: bool,
    pub support: Option<SupportDocument>,
}

/// A validation finding of a RIPS JSON (`core/findDetailsByFile`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDetail {
    pub code: String,
    pub kind: String,
    pub field: String,
    pub detail: String,
}
