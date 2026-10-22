//! Download clean-up (`DescargarXML.descargarArchivoXML`, TF-06) and validation.

use std::sync::LazyLock;

use regex::Regex;

use super::FevxmlError;
use super::dom;
use super::text::{decode, group, java_trim, re, remove_all, replace_first};

static PREPAID_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"<cac:PrepaidPayment(?-u:\b)[^>]*>.*?</cac:PrepaidPayment>"));
static PREPAID_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(<cbc:ID)(?:\s+[^>]*)?(>\s*[^<]*\s*</cbc:ID>)"));
static INSTRUCTION_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"<cbc:InstructionID(?-u:\b)[^>]*>(.*?)</cbc:InstructionID>"));

/// Pattern of one `AdditionalInformation` block named `name`.
pub(crate) fn info_pattern(name: &str) -> String {
    format!(
        r"<AdditionalInformation>\s*<Name>\s*{}\s*</Name>\s*<Value(?:\s+[^>]*)?>.*?</Value>\s*</AdditionalInformation>",
        regex::escape(java_trim(name))
    )
}

static PAGO_RES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        "COPAGO",
        "CUOTA_MODERADORA",
        "PAGOS_COMPARTIDOS",
        "ANTICIPOS",
    ]
    .iter()
    .map(|n| re(&format!(r"\s*{}", info_pattern(n))))
    .collect()
});

pub(crate) fn prepaid_re() -> &'static Regex {
    &PREPAID_RE
}

/// `eliminarCamposPagoSectorSalud`: drops the COPAGO / CUOTA_MODERADORA /
/// PAGOS_COMPARTIDOS / ANTICIPOS blocks together with the preceding whitespace.
pub(crate) fn eliminar_campos_pago(s: &str) -> String {
    let mut out = s.to_owned();
    for r in PAGO_RES.iter() {
        out = remove_all(r, &out);
    }
    out
}

/// `schemeID` of a `PrepaidPayment` block from its `cbc:InstructionID`.
pub(crate) fn scheme_id_for(block: &str) -> &'static str {
    let concepto = INSTRUCTION_RE
        .captures(block)
        .map(|m| java_trim(group(&m, 1)).to_uppercase())
        .unwrap_or_default()
        .replace('_', " ");
    if concepto.contains("ANTICIPO") {
        "04"
    } else if concepto.contains("PAGO COMPARTIDO") || concepto.contains("PAGOS COMPARTIDOS") {
        "03"
    } else if concepto.contains("CUOTA") {
        "02"
    } else if concepto.contains("COPAGO") {
        "01"
    } else {
        "04"
    }
}

/// `asegurarSchemeIdPrepaidPayment`: sets `schemeID` on the first `cbc:ID` of
/// every `cac:PrepaidPayment` (replacing any existing attributes).
pub(crate) fn scheme_id_prepaid(s: &str) -> String {
    PREPAID_RE
        .replace_all(s, |caps: &regex::Captures<'_>| {
            let block = group(caps, 0);
            let id = scheme_id_for(block);
            replace_first(&PREPAID_ID_RE, block, |m| {
                format!("{} schemeID=\"{}\"{}", group(m, 1), id, group(m, 2))
            })
            .unwrap_or_else(|| block.to_owned())
        })
        .into_owned()
}

/// Applies the post-download edits (TF-06): removes the payment fields of the
/// health sector and sets the `schemeID` of the prepaid payments. Unchanged
/// input is returned as is (even when it is not valid UTF-8).
pub fn normalize_download(b: &[u8]) -> Vec<u8> {
    let s = decode(b);
    let out = scheme_id_prepaid(&eliminar_campos_pago(&s));
    if out == s {
        b.to_vec()
    } else {
        out.into_bytes()
    }
}

/// Checks a download before accepting it: at least 40 bytes and a well-formed
/// document with a root element.
pub fn validate(b: &[u8]) -> Result<(), FevxmlError> {
    if b.len() < 40 {
        return Err(FevxmlError::XmlCorto(b.len()));
    }
    let s = decode(b);
    match dom::has_root(&s) {
        Ok(true) => Ok(()),
        Ok(false) => Err(FevxmlError::XmlSinRaiz),
        Err(e) => Err(FevxmlError::XmlMalFormado(e)),
    }
}
