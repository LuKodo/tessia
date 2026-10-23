//! `prepararXmlMinisterioEvento`: the edits applied to the XML before it is sent
//! to the ministry (CUV-06..08). Order and non-idempotent behaviors follow the
//! Java code as ported in Go.

use std::sync::LazyLock;

use regex::Regex;

use super::download::{eliminar_campos_pago, info_pattern, scheme_id_prepaid};
use super::period::ajustar_periodo;
use super::text::{decode, escape, group, java_trim, re, remove_all, replace_first};
use super::{CONTRATO_LEGACY, FevxmlError, MODALIDAD_PAGO};

static COLLECTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(r#"(<Collection(?-u:\b)[^>]*schemeName\s*=\s*"Usuario"[^>]*>)(.*?)(</Collection>)"#)
});
pub(crate) static CONTRATO_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(<AdditionalInformation>\s*<Name>\s*NUMERO_CONTRATO\s*</Name>\s*<Value(?:\s+[^>]*)?>)(.*?)(</Value>\s*</AdditionalInformation>)",
    )
});
static COBERTURA_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(<AdditionalInformation>\s*<Name>\s*COBERTURA_PLAN_BENEFICIOS\s*</Name>\s*)<Value([^>]*)>(.*?)</Value>(\s*</AdditionalInformation>)",
    )
});
static SCHEME_ID_RE: LazyLock<Regex> = LazyLock::new(|| re(r#"schemeID\s*=\s*"([^"]*)""#));
static CAMPO22_RE: LazyLock<Regex> = LazyLock::new(|| re(&info_pattern("22")));
static SIN_CONTRATO_RE: LazyLock<Regex> =
    LazyLock::new(|| re(&info_pattern("FACTURA_SIN_CONTRATO")));
static POLIZA_RE: LazyLock<Regex> =
    LazyLock::new(|| re(&format!("({})", info_pattern("NUMERO_POLIZA"))));
static INFO_RES: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    [
        "CODIGO_PRESTADOR",
        "MODALIDAD_PAGO",
        "COBERTURA_PLAN_BENEFICIOS",
        "NUMERO_CONTRATO",
        "NUMERO_POLIZA",
    ]
    .iter()
    .map(|n| (*n, re(&info_pattern(n))))
    .collect()
});

/// `existeAdditionalInformationEnTexto` for one of the known field names.
fn has_info(s: &str, name: &str) -> bool {
    INFO_RES
        .iter()
        .find(|(n, _)| *n == name)
        .is_some_and(|(_, r)| r.is_match(s))
}

/// Arguments of `prepararXmlMinisterioEvento`.
#[derive(Debug, Clone, Default)]
pub struct Params {
    /// 10-digit REPS code (Java constant `1300185002`).
    pub codigo_prestador: String,
    /// Replaces NUMERO_CONTRATO when it is 22382, and is inserted when missing.
    pub contrato: String,
    /// The user's tipoUsuario: `01` contributivo, `04` subsidiado.
    pub regimen: String,
    /// Start date forced on every period (normally `2025-01-01`).
    pub periodo_inicio: String,
}

/// Result of [`prepare_ministerio`].
#[derive(Debug)]
pub struct Prepared {
    /// The edited XML, or the input bytes when nothing changed.
    pub xml: Vec<u8>,
    /// Whether `xml` differs from the input.
    pub changed: bool,
    /// Set (to [`FevxmlError::Periodo`]) when the period step failed; the edits
    /// before that step are kept in `xml`, as Java had already written them.
    pub period_error: Option<FevxmlError>,
}

/// Result of replacing NUMERO_CONTRATO.
pub(crate) enum ContratoOutcome {
    Modificado,
    Diferente,
    Sin,
}

/// `reemplazarNumeroContratoSiValorActual(..., "22382", contrato)`.
pub(crate) fn replace_contrato(s: &str, contrato: &str) -> (String, ContratoOutcome) {
    let Some(caps) = CONTRATO_RE.captures(s) else {
        return (s.to_owned(), ContratoOutcome::Sin);
    };
    if java_trim(group(&caps, 2)) != CONTRATO_LEGACY {
        return (s.to_owned(), ContratoOutcome::Diferente);
    }
    let Some(whole) = caps.get(0) else {
        return (s.to_owned(), ContratoOutcome::Sin);
    };
    let out = format!(
        "{}{}{}{}{}",
        &s[..whole.start()],
        group(&caps, 1),
        escape(java_trim(contrato)),
        group(&caps, 3),
        &s[whole.end()..]
    );
    (out, ContratoOutcome::Modificado)
}

fn simple_value(v: &str) -> String {
    format!("<Value>{}</Value>", escape(java_trim(v)))
}

fn cobertura(regimen: &str) -> Option<(&'static str, &'static str)> {
    match regimen {
        "01" => Some((
            "16",
            "Plan de beneficios en salud financiado con UPC contributivo",
        )),
        "04" => Some((
            "17",
            "Plan de beneficios en salud financiado con UPC subsidiado",
        )),
        _ => None,
    }
}

fn asegurar_en_coleccion(s: &str, name: &str, value_xml: &str) -> String {
    if has_info(s, name) {
        return s.to_owned();
    }
    let block = format!(
        "\n                                    <AdditionalInformation><Name>{}</Name>{}</AdditionalInformation>",
        escape(name),
        value_xml
    );
    replace_first(&COLLECTION_RE, s, |m| {
        format!("{}{}{}{}", group(m, 1), group(m, 2), block, group(m, 3))
    })
    .unwrap_or_else(|| s.to_owned())
}

fn asegurar_campos_salud(s: &str, p: &Params) -> String {
    let mut s = asegurar_en_coleccion(s, "CODIGO_PRESTADOR", &simple_value(&p.codigo_prestador));
    s = asegurar_en_coleccion(
        &s,
        "MODALIDAD_PAGO",
        &format!(
            r#"<Value schemeID="04" schemeName="salud_modalidad_pago.gc">{}</Value>"#,
            escape(java_trim(MODALIDAD_PAGO))
        ),
    );
    if let Some((code, desc)) = cobertura(java_trim(&p.regimen)) {
        s = asegurar_en_coleccion(
            &s,
            "COBERTURA_PLAN_BENEFICIOS",
            &format!(r#"<Value schemeID="{code}" schemeName="salud_cobertura.gc">{desc}</Value>"#),
        );
    }
    s = asegurar_en_coleccion(&s, "NUMERO_CONTRATO", &simple_value(&p.contrato));
    s = asegurar_en_coleccion(&s, "NUMERO_POLIZA", &simple_value(""));
    scheme_id_prepaid(&eliminar_campos_pago(&s))
}

fn ajustar_cobertura(s: &str, regimen: &str) -> String {
    let Some((code, desc)) = cobertura(java_trim(regimen)) else {
        return s.to_owned();
    };
    replace_first(&COBERTURA_RE, s, |m| {
        let actual = java_trim(group(m, 3));
        let scheme = SCHEME_ID_RE
            .captures(group(m, 2))
            .map(|sm| java_trim(group(&sm, 1)).to_owned())
            .unwrap_or_default();
        if scheme == "02"
            || (scheme.is_empty() && actual == "02")
            || (scheme == code && actual == desc)
        {
            return group(m, 0).to_owned();
        }
        format!(
            r#"{}<Value schemeID="{code}" schemeName="salud_cobertura.gc">{desc}</Value>{}"#,
            group(m, 1),
            group(m, 4)
        )
    })
    .unwrap_or_else(|| s.to_owned())
}

const CAMPO_FACTURA_SIN_CONTRATO: &str = "\n                <AdditionalInformation>\n                  <Name>FACTURA_SIN_CONTRATO</Name>\n                  <Value></Value>\n                </AdditionalInformation>";

/// Removes field 22 and re-inserts FACTURA_SIN_CONTRATO after NUMERO_POLIZA
/// (or in place of the existing one when there is no NUMERO_POLIZA).
fn asegurar_factura_sin_contrato(s: &str) -> String {
    let s = remove_all(&CAMPO22_RE, s);
    if POLIZA_RE.is_match(&s) {
        let s = remove_all(&SIN_CONTRATO_RE, &s);
        return replace_first(&POLIZA_RE, &s, |m| {
            format!("{}{}", group(m, 1), CAMPO_FACTURA_SIN_CONTRATO)
        })
        .unwrap_or(s);
    }
    SIN_CONTRATO_RE
        .replace_all(&s, regex::NoExpand(CAMPO_FACTURA_SIN_CONTRATO))
        .into_owned()
}

/// Applies `prepararXmlMinisterioEvento` (with an empty end date) to `b`.
///
/// Fails with [`FevxmlError::SinContrato`] before any edit when the XML has no
/// NUMERO_CONTRATO. A failing period step does not abort: see [`Prepared`].
pub fn prepare_ministerio(b: &[u8], p: &Params) -> Result<Prepared, FevxmlError> {
    let s = decode(b);
    if !has_info(&s, "NUMERO_CONTRATO") {
        return Err(FevxmlError::SinContrato);
    }
    let mut out = replace_contrato(&s, &p.contrato).0;
    out = asegurar_campos_salud(&out, p);
    out = ajustar_cobertura(&out, &p.regimen);
    out = asegurar_factura_sin_contrato(&out);
    out = eliminar_campos_pago(&out);
    out = scheme_id_prepaid(&out);
    let (out, period_error) = match ajustar_periodo(&out, java_trim(&p.periodo_inicio)) {
        Ok(o) => (o, None),
        Err(e) => (out, Some(e)),
    };
    let changed = out != s;
    let xml = if changed {
        out.into_bytes()
    } else {
        b.to_vec()
    };
    Ok(Prepared {
        xml,
        changed,
        period_error,
    })
}
