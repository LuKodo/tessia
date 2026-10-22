//! Text edits and readers for the FEV XML (the DIAN `AttachedDocument`
//! downloaded from The Factory), ported from the Go package `internal/fevxml`
//! and the Java `ModificarXMLConCDATA`.
//!
//! The invoice sits inside a `cbc:Description` CDATA, and the legacy tool edited
//! the raw text with regular expressions. This module does the same, byte for
//! byte: no DOM is ever re-serialized. A DOM-like reader is only used for
//! read-only extraction ([`invoice_lines`], [`validate`]).
//!
//! The module has no dependency on the rest of the crate.

// The consumers (thefactory, fev, cuv, conciliar, radicacion) are ported later;
// drop this allow once they use the API.
#![allow(dead_code, unused_imports)]

mod ajuste;
mod dom;
mod download;
mod invoice_total;
mod lines;
mod ministerio;
mod period;
mod text;
mod zipio;

pub use ajuste::{
    ContratoResult, cucon, cuota_moderadora, numero_convenio, parse_decimal_literal, read_total,
};
pub use download::{normalize_download, validate};
pub use invoice_total::{
    InvoiceXmlFile, XmlState, existing_xml_pdf_zip, format_total, invoice_total,
    locate_invoice_xml, storage_amount, xml_pdf_zip_paths,
};
pub use lines::{InvoiceLine, invoice_lines};
pub use ministerio::{Params, Prepared, prepare_ministerio};
pub use zipio::{MAX_ENTRY_BYTES, build_xml_pdf_zip, first_xml_entry, read_entry, replace_entry};

use thiserror::Error;

/// `PeriodoInicioMinisterio`: start date forced on every period.
pub const PERIODO_INICIO_MINISTERIO: &str = "2025-01-01";

/// `ModalidadPago` passed by `EnvioRIPS_Financiera_Extra`.
pub const MODALIDAD_PAGO: &str = "Pago por evento";

/// Contract value that [`prepare_ministerio`] and [`cucon`] replace.
pub const CONTRATO_LEGACY: &str = "22382";

/// Reason code reported when the XML has no `NUMERO_CONTRATO`.
pub const REASON_XML_SIN_CONTRATO: &str = "XML_SIN_CONTRATO";

/// Reason code reported when the invoice total cannot be read from the XML.
pub const REASON_XML_SIN_VALOR: &str = "XML_SIN_VALOR";

/// Errors of the XML library. The Spanish messages are a stored/printed
/// contract (they surface in CUV and conciliation output).
#[derive(Debug, Error)]
pub enum FevxmlError {
    /// The XML has no `NUMERO_CONTRATO`; Java rebuilt the health-sector
    /// extension through a DOM round trip, which is not ported.
    #[error("fevxml: XML sin NUMERO_CONTRATO (Java: modificarXMLEvento, no portado)")]
    SinContrato,
    /// A period still has another start date after the edit. Earlier edits are kept.
    #[error(
        "fevxml: no se pudo garantizar la fecha inicial del periodo en todos los periodos del XML"
    )]
    Periodo,
    /// The downloaded XML is shorter than 40 bytes.
    #[error("fevxml: XML de {0} bytes (< 40)")]
    XmlCorto(usize),
    /// The XML is not well formed.
    #[error("fevxml: XML mal formado: {0}")]
    XmlMalFormado(String),
    /// The XML has no root element.
    #[error("fevxml: XML sin elemento raiz")]
    XmlSinRaiz,
    /// The invoice XML could not be parsed.
    #[error("el XML de factura no es valido: {0}")]
    XmlInvalido(String),
    /// The invoice has no reconcilable lines.
    #[error("el XML no contiene lineas de factura conciliables")]
    SinLineas,
    /// A line misses its identifier or has a non-positive/invalid number.
    #[error("linea XML incompleta: {0}")]
    LineaIncompleta(String),
    /// `|quantity x price - total|` exceeds the 0.01 tolerance.
    #[error("la cantidad y el precio XML no forman el total de la linea {0}")]
    LineaInconsistente(String),
    /// A ZIP entry is larger than [`MAX_ENTRY_BYTES`].
    #[error("fevxml: la entrada {0} del ZIP supera el limite de {1} bytes")]
    EntradaDemasiadoGrande(String, u64),
    /// ZIP container error.
    #[error("fevxml: zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// I/O error while reading or writing an entry.
    #[error("fevxml: io: {0}")]
    Io(#[from] std::io::Error),
}

impl FevxmlError {
    /// Stable reason code for the errors that surface as one in the CLI.
    pub fn reason_code(&self) -> Option<&'static str> {
        match self {
            Self::SinContrato => Some(REASON_XML_SIN_CONTRATO),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
