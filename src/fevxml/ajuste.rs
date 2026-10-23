//! Standalone XML support tools (FEV `ajustar` / `verificar`) and the invoice
//! value readers:
//!
//! - `cucon`: `Vista_XML_CUCON` ("Corregir XML de contrato a CUCON").
//! - `cuota_moderadora`: `Vista_AjustarXMLCuotaModeradora`.
//! - `read_total` / `numero_convenio`: the value check of
//!   `validarYConciliarValorAntesJson`.

use std::sync::LazyLock;

use regex::Regex;
use rust_decimal::Decimal;

use super::download::{prepaid_re, scheme_id_for};
use super::ministerio::{ContratoOutcome, replace_contrato};
use super::text::{decode, group, java_trim, re, remove_all, replace_first};

/// The `Vista_XML_CUCON` counter an XML falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContratoResult {
    /// NUMERO_CONTRATO was 22382 and has been replaced.
    Modificado,
    /// NUMERO_CONTRATO has another value.
    ContratoDiferente,
    /// The XML has no NUMERO_CONTRATO.
    SinContrato,
}

impl ContratoResult {
    /// Counter name as printed by the Go tool.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Modificado => "modificado",
            Self::ContratoDiferente => "contrato_diferente",
            Self::SinContrato => "sin_contrato",
        }
    }
}

/// Parses a Java `new BigDecimal(String)` literal (sign, digits with optional
/// `.`, optional exponent; no surrounding whitespace). Returns `None` when
/// invalid or when the value does not fit `Decimal` (96-bit mantissa, scale <= 28).
pub fn parse_decimal_literal(s: &str) -> Option<Decimal> {
    let (neg, rest) = match s.as_bytes().first()? {
        b'+' => (false, &s[1..]),
        b'-' => (true, &s[1..]),
        _ => (false, s),
    };
    let rest = rest.replace('E', "e");
    let (mant, exp) = match rest.split_once('e') {
        Some((m, e)) => (m, Some(e)),
        None => (rest.as_str(), None),
    };
    let (int_part, frac_part) = mant.split_once('.').unwrap_or((mant, ""));
    let digits = format!("{int_part}{frac_part}");
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut scale = i64::try_from(frac_part.len()).ok()?;
    if let Some(e) = exp {
        if e.starts_with("+-") || e.starts_with("-+") {
            return None;
        }
        scale -= i64::from(e.parse::<i32>().ok()?);
    }
    let trimmed = digits.trim_start_matches('0');
    let mut unscaled: i128 = if trimmed.is_empty() {
        0
    } else {
        trimmed
            .parse::<u128>()
            .ok()
            .and_then(|u| i128::try_from(u).ok())?
    };
    if scale < 0 {
        let factor = 10i128.checked_pow(u32::try_from(-scale).ok()?)?;
        unscaled = unscaled.checked_mul(factor)?;
        scale = 0;
    }
    if neg {
        unscaled = -unscaled;
    }
    Decimal::try_from_i128_with_scale(unscaled, u32::try_from(scale).ok()?).ok()
}

/// `new BigDecimal(trim)`, zero when empty or invalid.
fn parse_decimal(v: &str) -> Decimal {
    parse_decimal_literal(java_trim(v)).unwrap_or(Decimal::ZERO)
}

/// Replaces NUMERO_CONTRATO = 22382 with `cucon`, like
/// `reemplazarNumeroContratoSiValorActual(..., "22382", cucon)`. The output is
/// the input bytes when nothing was replaced. Java rewrote the file even when
/// `cucon` was 22382; the result is then [`ContratoResult::Modificado`] with
/// unchanged bytes.
pub fn cucon(b: &[u8], cucon: &str) -> (Vec<u8>, ContratoResult) {
    let s = decode(b);
    let (out, res) = replace_contrato(&s, cucon);
    let res = match res {
        ContratoOutcome::Modificado => ContratoResult::Modificado,
        ContratoOutcome::Diferente => ContratoResult::ContratoDiferente,
        ContratoOutcome::Sin => ContratoResult::SinContrato,
    };
    if out == s {
        (b.to_vec(), res)
    } else {
        (out.into_bytes(), res)
    }
}

static CUOTA_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(<cbc:ID)(?:\s+[^>]*)?(>\s*([^<]+?)\s*</cbc:ID>)"));
static PAID_AMOUNT_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"<cbc:PaidAmount(?-u:\b)[^>]*>([^<]*)</cbc:PaidAmount>"));
static PREPAID_AMOUNT_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"<cbc:PrepaidAmount(?-u:\b)[^>]*>([^<]*)</cbc:PrepaidAmount>"));
static ANTICIPOS_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\s*<AdditionalInformation>\s*<Name>\s*ANTICIPOS\s*</Name>\s*<Value(?:\s+[^>]*)?>([^<]*)</Value>\s*</AdditionalInformation>",
    )
});

/// Result of [`cuota_moderadora`].
#[derive(Debug)]
pub struct CuotaResult {
    /// The adjusted XML, or the input bytes when unchanged.
    pub xml: Vec<u8>,
    /// Whether the XML changed.
    pub changed: bool,
    /// The `ADVERTENCIA` lines Java printed.
    pub warnings: Vec<String>,
}

/// `Vista_AjustarXMLCuotaModeradora.procesarXml`: when the XML has an anticipo
/// to legalize (ANTICIPOS != 0, or PrepaidPayment PaidAmounts > 0) every
/// PrepaidPayment ID gets its `schemeID` and the ANTICIPOS field is removed.
/// `name` is the file name used in the warnings.
pub fn cuota_moderadora(b: &[u8], name: &str) -> CuotaResult {
    let s = decode(b);
    if !tiene_anticipo(&s) {
        return CuotaResult {
            xml: b.to_vec(),
            changed: false,
            warnings: Vec::new(),
        };
    }
    let out = prepaid_re()
        .replace_all(&s, |caps: &regex::Captures<'_>| {
            let block = group(caps, 0);
            replace_first(&CUOTA_ID_RE, block, |m| {
                format!(
                    "{} schemeID=\"{}\"{}",
                    group(m, 1),
                    scheme_id_for(block),
                    group(m, 2)
                )
            })
            .unwrap_or_else(|| block.to_owned())
        })
        .into_owned();
    let out = remove_all(&ANTICIPOS_RE, &out);
    let warnings = verificar_prepaid_amount(&out, name);
    if out == s {
        CuotaResult {
            xml: b.to_vec(),
            changed: false,
            warnings,
        }
    } else {
        CuotaResult {
            xml: out.into_bytes(),
            changed: true,
            warnings,
        }
    }
}

fn tiene_anticipo(s: &str) -> bool {
    if let Some(m) = ANTICIPOS_RE.captures(s) {
        let v = java_trim(group(&m, 1));
        if !v.is_empty() && !es_valor_cero(v) {
            return true;
        }
    }
    suma_paid_amount(s) > Decimal::ZERO
}

fn es_valor_cero(v: &str) -> bool {
    match parse_decimal_literal(java_trim(v)) {
        Some(d) => d.is_zero(),
        None => java_trim(v) == "0",
    }
}

fn suma_paid_amount(s: &str) -> Decimal {
    let mut total = Decimal::ZERO;
    for block in prepaid_re().find_iter(s) {
        if let Some(m) = PAID_AMOUNT_RE.captures(block.as_str()) {
            total = total
                .checked_add(parse_decimal(group(&m, 1)))
                .unwrap_or(total);
        }
    }
    total
}

fn verificar_prepaid_amount(s: &str, name: &str) -> Vec<String> {
    let suma = suma_paid_amount(s);
    if suma <= Decimal::ZERO {
        return Vec::new();
    }
    let Some(m) = PREPAID_AMOUNT_RE.captures(s) else {
        return vec![format!(
            "ADVERTENCIA {name}: no se encontro cbc:PrepaidAmount para anticipos por {suma}"
        )];
    };
    let p = parse_decimal(group(&m, 1));
    if p != suma {
        return vec![format!(
            "ADVERTENCIA {name}: PrepaidAmount={p} no coincide con suma PaidAmount={suma}"
        )];
    }
    Vec::new()
}

static CDATA_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"<cbc:Description>\s*<!\[CDATA\[(.*?)\]\]>\s*</cbc:Description>"));
static TOTAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"<cac:LegalMonetaryTotal(?-u:\b)[^>]*>.*?<cbc:(?:PayableAmount|TaxInclusiveAmount)(?-u:\b)[^>]*>\s*([0-9.,]+)\s*</cbc:(?:PayableAmount|TaxInclusiveAmount)>.*?</cac:LegalMonetaryTotal>",
    )
});

/// Invoice value of `extraerValorTotalXmlFactura`: the first PayableAmount or
/// TaxInclusiveAmount (whichever comes first) of the LegalMonetaryTotal inside
/// the CDATA invoice (or the whole XML without CDATA). A value with both `,`
/// and `.` loses its commas; a lone `,` is the decimal separator.
pub fn read_total(b: &[u8]) -> Option<Decimal> {
    let full = decode(b);
    let s = CDATA_RE
        .captures(&full)
        .map(|m| group(&m, 1).to_owned())
        .unwrap_or(full);
    let m = TOTAL_RE.captures(&s)?;
    let v = java_trim(group(&m, 1));
    let v = if v.contains(',') && v.contains('.') {
        v.replace(',', "")
    } else if v.contains(',') {
        v.replace(',', ".")
    } else {
        v.to_owned()
    };
    parse_decimal_literal(&v)
}

/// Parses a convenio "vlr. total" like `numeroConvenio`: `$` and spaces are
/// removed; with both separators `.` is thousands and `,` the decimal; a lone
/// `,` is the decimal.
pub fn numero_convenio(s: &str) -> Option<Decimal> {
    let v = java_trim(s);
    if v.is_empty() {
        return None;
    }
    let v = v.replace(['$', ' '], "");
    let v = if v.contains(',') && v.contains('.') {
        v.replace('.', "").replace(',', ".")
    } else if v.contains(',') {
        v.replace(',', ".")
    } else {
        v
    };
    parse_decimal_literal(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_java_literals() {
        let p = |s: &str| parse_decimal_literal(s).map(|d| d.to_string());
        assert_eq!(p("1e3").as_deref(), Some("1000"));
        assert_eq!(p("007.10").as_deref(), Some("7.10"));
        assert_eq!(p("5.").as_deref(), Some("5"));
        assert_eq!(p(".5").as_deref(), Some("0.5"));
        assert_eq!(p("-20").as_deref(), Some("-20"));
        assert_eq!(p("12e-2").as_deref(), Some("0.12"));
        assert_eq!(p("1.2.3"), None);
        assert_eq!(p("abc"), None);
        assert_eq!(p(""), None);
        assert_eq!(p("1e"), None);
        assert_eq!(p(" 1"), None);
    }
}
