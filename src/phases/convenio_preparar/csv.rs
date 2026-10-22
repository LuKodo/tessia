//! Reader of the Medicar convenio export (port of Go `convenio/csv.go`).
//!
//! UTF-8 (BOM tolerated), CRLF or LF, `|` or `;` separated; the first
//! non-empty line is the header and columns are matched by *normalized* header
//! name (lower case, accents removed, spaces collapsed), so the column order
//! does not matter. Rows whose field count differs from the header are skipped
//! and their 1-based line numbers reported. Values are trimmed and
//! load-normalized (`123,00` -> `123`, scientific notation -> plain digits).
//!
//! The error texts are Go's, unchanged.

use std::collections::HashMap;
use std::io::BufRead;
use std::sync::LazyLock;

use anyhow::{Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::domain::normalize_invoice;

// Canonical (normalized) header names.
const COL_NIT: &str = "nit";
const COL_PLAN: &str = "plan";
const COL_SUBPLAN: &str = "subplan";
const COL_INVOICE: &str = "factura";
const COL_DOC_TYPE: &str = "tipo de identificacion";
const COL_DOC_NUMBER: &str = "no. documento";
const COL_BENEFICIARY: &str = "beneficiario";
const COL_POINT_OF_SALE: &str = "pto. venta";
const COL_FORMULA: &str = "no. formula";
const COL_SSC: &str = "no. ssc";
const COL_FORMULA_TYPE: &str = "tipo formula";
const COL_DATE: &str = "fecha";
const COL_LOT: &str = "lote";
const COL_PLU: &str = "plu";
const COL_ARTICLE: &str = "articulo";
const COL_QUANTITY: &str = "cantidad";
const COL_UNIT_VALUE: &str = "vlr. unitario";
const COL_UNIT_COST: &str = "costo unitario";
const COL_TOTAL_VALUE: &str = "vlr. total";
const COL_COPAY: &str = "cuota";
const COL_AUTHORIZATION: &str = "numero autorizacion";
const COL_MIPRES: &str = "mipres";
const COL_CUM: &str = "cum";
const COL_DIAGNOSIS: &str = "codigo diagnostico";
const COL_DIAGNOSIS_NAME: &str = "nombre diagnostico";
const COL_CAUSAL_EXENTO: &str = "causal exento";
const COL_IPS: &str = "ips";
const COL_PRESCRIBER_ID: &str = "id medico";
const COL_PRESCRIBER_NAME: &str = "nombre medico";
const COL_GENDER: &str = "genero paciente";
const COL_BIRTH_DATE: &str = "fecha nacimiento";
const COL_DEPARTMENT: &str = "departamento";
const COL_MUNICIPALITY: &str = "municipio";

// Optional columns of a manual "prepared format" export.
const COL_CLASSIFICATION: &str = "clasificacion";
const COL_FILE_FORM: &str = "med-forma farmaceutica rips";
const COL_FILE_CONCENTRATION: &str = "med-concentracion medicamento rips";
const COL_FILE_UNIT: &str = "med-unidad medida rips";

/// Columns that must be present in the header.
pub const REQUIRED_COLUMNS: [&str; 11] = [
    COL_INVOICE,
    COL_DOC_TYPE,
    COL_DOC_NUMBER,
    COL_DATE,
    COL_PLU,
    COL_ARTICLE,
    COL_QUANTITY,
    COL_UNIT_VALUE,
    COL_TOTAL_VALUE,
    COL_COPAY,
    COL_CUM,
];

fn is_blank(text: &str) -> bool {
    text.is_empty()
}

/// One dispensed item of the convenio plus the enrichment added by the
/// preparation. The serde names are Go's JSON tags (the persisted shape).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    #[serde(rename = "source_line", default)]
    pub source_line: usize,
    #[serde(rename = "nit", default, skip_serializing_if = "is_blank")]
    pub nit: String,
    #[serde(rename = "plan", default, skip_serializing_if = "is_blank")]
    pub plan: String,
    #[serde(rename = "subplan", default, skip_serializing_if = "is_blank")]
    pub subplan: String,
    /// Normalized (upper case, no dashes).
    #[serde(rename = "factura", default)]
    pub invoice: String,
    /// As exported.
    #[serde(rename = "factura_raw", default)]
    pub invoice_raw: String,
    #[serde(rename = "tipo_documento", default)]
    pub doc_type: String,
    #[serde(rename = "numero_documento", default)]
    pub doc_number: String,
    #[serde(rename = "beneficiario", default, skip_serializing_if = "is_blank")]
    pub beneficiary: String,
    #[serde(rename = "punto_venta", default, skip_serializing_if = "is_blank")]
    pub point_of_sale: String,
    #[serde(rename = "numero_formula", default, skip_serializing_if = "is_blank")]
    pub formula_number: String,
    #[serde(rename = "ssc", default, skip_serializing_if = "is_blank")]
    pub ssc: String,
    #[serde(rename = "tipo_formula", default, skip_serializing_if = "is_blank")]
    pub formula_type: String,
    #[serde(rename = "fecha", default)]
    pub date: String,
    #[serde(rename = "lote", default, skip_serializing_if = "is_blank")]
    pub lot: String,
    #[serde(rename = "plu", default)]
    pub plu: String,
    #[serde(rename = "articulo", default)]
    pub article: String,
    #[serde(rename = "cantidad", default)]
    pub quantity: String,
    #[serde(rename = "valor_unitario", default)]
    pub unit_value: String,
    #[serde(rename = "costo_unitario", default, skip_serializing_if = "is_blank")]
    pub unit_cost: String,
    #[serde(rename = "valor_total", default)]
    pub total_value: String,
    #[serde(rename = "cuota", default)]
    pub copay: String,
    #[serde(
        rename = "numero_autorizacion",
        default,
        skip_serializing_if = "is_blank"
    )]
    pub authorization: String,
    #[serde(rename = "mipres", default, skip_serializing_if = "is_blank")]
    pub mipres: String,
    /// CUM as exported.
    #[serde(rename = "cum_convenio", default)]
    pub agreement_cum: String,
    #[serde(
        rename = "codigo_diagnostico",
        default,
        skip_serializing_if = "is_blank"
    )]
    pub diagnosis_code: String,
    #[serde(
        rename = "nombre_diagnostico",
        default,
        skip_serializing_if = "is_blank"
    )]
    pub diagnosis_name: String,
    #[serde(rename = "causal_exento", default, skip_serializing_if = "is_blank")]
    pub causal_exento: String,
    #[serde(rename = "ips", default, skip_serializing_if = "is_blank")]
    pub ips: String,
    #[serde(rename = "id_medico", default, skip_serializing_if = "is_blank")]
    pub prescriber_id: String,
    #[serde(rename = "nombre_medico", default, skip_serializing_if = "is_blank")]
    pub prescriber_name: String,
    #[serde(rename = "genero", default, skip_serializing_if = "is_blank")]
    pub gender: String,
    #[serde(rename = "fecha_nacimiento", default, skip_serializing_if = "is_blank")]
    pub birth_date: String,
    #[serde(rename = "departamento", default, skip_serializing_if = "is_blank")]
    pub department: String,
    #[serde(rename = "municipio", default, skip_serializing_if = "is_blank")]
    pub municipality: String,
    /// The file's own CLASIFICACION column (prepared-format exports only).
    #[serde(rename = "clasificacion", default, skip_serializing_if = "is_blank")]
    pub classification: String,

    /// Billing CUM (catalog); the exported CUM until enriched.
    #[serde(rename = "cum", default)]
    pub cum: String,
    /// 714 medicine, 799 other service.
    #[serde(rename = "tipo_producto", default, skip_serializing_if = "is_blank")]
    pub product_type: String,
    #[serde(
        rename = "forma_farmaceutica",
        default,
        skip_serializing_if = "is_blank"
    )]
    pub pharmaceutical_form: String,
    #[serde(rename = "concentracion", default, skip_serializing_if = "is_blank")]
    pub concentration: String,
    #[serde(rename = "unidad_medida", default, skip_serializing_if = "is_blank")]
    pub measure_unit: String,
    #[serde(rename = "precio_pactado", default, skip_serializing_if = "is_blank")]
    pub agreed_price: String,
}

/// Outcome of [`read_csv`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadResult {
    pub lines: Vec<Line>,
    /// Non-empty data lines with a wrong field count.
    pub skipped: usize,
    /// 1-based numbers of the skipped lines.
    pub skipped_lines: Vec<usize>,
    pub separator: char,
    /// [`has_prepared_format`] of `lines`.
    pub prepared_format: bool,
}

/// Reads a Medicar convenio export.
pub fn read_csv<R: BufRead>(mut reader: R) -> Result<ReadResult> {
    let mut result = ReadResult::default();
    let mut index: Option<HashMap<String, usize>> = None;
    let mut width = 0;
    let mut line_no = 0;
    let mut raw = Vec::new();
    loop {
        raw.clear();
        let read = reader
            .read_until(b'\n', &mut raw)
            .map_err(|err| anyhow::anyhow!("convenio: read CSV: {err}"))?;
        if read == 0 {
            break;
        }
        line_no += 1;
        let mut text = String::from_utf8_lossy(&raw).into_owned();
        if text.ends_with('\n') {
            text.pop();
        }
        let mut text = text.trim_end_matches('\r').to_owned();
        if line_no == 1 {
            if let Some(stripped) = text.strip_prefix('\u{feff}') {
                text = stripped.to_owned();
            }
        }
        if text.trim().is_empty() {
            continue;
        }
        let Some(columns) = index.as_ref() else {
            let (header_index, header_width, separator) = parse_header(&text)?;
            result.separator = separator;
            width = header_width;
            index = Some(header_index);
            continue;
        };
        let fields: Vec<&str> = text.split(result.separator).collect();
        if fields.len() != width {
            result.skipped += 1;
            result.skipped_lines.push(line_no);
            continue;
        }
        result.lines.push(build_line(columns, &fields, line_no));
    }
    if index.is_none() {
        bail!("convenio: the CSV is empty");
    }
    result.prepared_format = has_prepared_format(&result.lines);
    Ok(result)
}

fn parse_header(text: &str) -> Result<(HashMap<String, usize>, usize, char)> {
    let Some(separator) = detect_separator(text) else {
        bail!("convenio: the header has no '|' or ';' separator");
    };
    let columns: Vec<&str> = text.split(separator).collect();
    let mut index = HashMap::with_capacity(columns.len());
    for (position, column) in columns.iter().enumerate() {
        let name = normalize_header(column);
        if !name.is_empty() {
            index.entry(name).or_insert(position);
        }
    }
    let missing: Vec<&str> = REQUIRED_COLUMNS
        .iter()
        .copied()
        .filter(|column| !index.contains_key(*column))
        .collect();
    if !missing.is_empty() {
        bail!("convenio: missing required columns: {}", missing.join(", "));
    }
    Ok((index, columns.len(), separator))
}

fn build_line(index: &HashMap<String, usize>, fields: &[&str], source_line: usize) -> Line {
    let get = |column: &str| -> String {
        index
            .get(column)
            .map(|&i| normalize_load_value(fields[i]))
            .unwrap_or_default()
    };
    let invoice_raw = get(COL_INVOICE);
    let agreement_cum = get(COL_CUM);
    Line {
        source_line,
        nit: get(COL_NIT),
        plan: get(COL_PLAN),
        subplan: get(COL_SUBPLAN),
        invoice: normalize_invoice(&invoice_raw),
        invoice_raw,
        doc_type: get(COL_DOC_TYPE),
        doc_number: get(COL_DOC_NUMBER),
        beneficiary: get(COL_BENEFICIARY),
        point_of_sale: get(COL_POINT_OF_SALE),
        formula_number: get(COL_FORMULA),
        ssc: get(COL_SSC),
        formula_type: get(COL_FORMULA_TYPE),
        date: get(COL_DATE),
        lot: get(COL_LOT),
        plu: get(COL_PLU),
        article: get(COL_ARTICLE),
        quantity: get(COL_QUANTITY),
        unit_value: get(COL_UNIT_VALUE),
        unit_cost: get(COL_UNIT_COST),
        total_value: get(COL_TOTAL_VALUE),
        copay: get(COL_COPAY),
        authorization: get(COL_AUTHORIZATION),
        mipres: get(COL_MIPRES),
        cum: agreement_cum.clone(),
        agreement_cum,
        diagnosis_code: get(COL_DIAGNOSIS),
        diagnosis_name: get(COL_DIAGNOSIS_NAME),
        causal_exento: get(COL_CAUSAL_EXENTO),
        ips: get(COL_IPS),
        prescriber_id: get(COL_PRESCRIBER_ID),
        prescriber_name: get(COL_PRESCRIBER_NAME),
        gender: get(COL_GENDER),
        birth_date: get(COL_BIRTH_DATE),
        department: get(COL_DEPARTMENT),
        municipality: get(COL_MUNICIPALITY),
        classification: get(COL_CLASSIFICATION),
        pharmaceutical_form: get(COL_FILE_FORM),
        concentration: get(COL_FILE_CONCENTRATION),
        measure_unit: get(COL_FILE_UNIT),
        ..Line::default()
    }
}

/// Legacy detection of a manual prepared-format load: some line of the whole
/// file has CLASIFICACION 714 or 799.
pub fn has_prepared_format(lines: &[Line]) -> bool {
    lines
        .iter()
        .any(|line| matches!(line.classification.trim(), "714" | "799"))
}

/// Prefers `|` (the export's separator) over `;`.
fn detect_separator(line: &str) -> Option<char> {
    if line.contains('|') {
        Some('|')
    } else if line.contains(';') {
        Some(';')
    } else {
        None
    }
}

fn strip_header_accent(c: char) -> char {
    match c {
        'á' | 'Á' => 'a',
        'é' | 'É' => 'e',
        'í' | 'Í' => 'i',
        'ó' | 'Ó' => 'o',
        'ú' | 'Ú' | 'ü' | 'Ü' => 'u',
        'ñ' | 'Ñ' => 'n',
        other => other,
    }
}

/// Lower-cases, strips accents and collapses whitespace.
pub fn normalize_header(text: &str) -> String {
    let stripped: String = text.chars().map(strip_header_accent).collect();
    stripped
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

static ZERO_CENTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-?[0-9]+,00$").unwrap_or_else(|_| unreachable!("constant")));
static SCIENTIFIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([+-]?)([0-9]+)(?:[.,]([0-9]+))?[eE]([+-]?[0-9]+)$")
        .unwrap_or_else(|_| unreachable!("constant"))
});

/// Trims and applies the legacy load rules: `123,00` -> `123`, and scientific
/// notation (Excel damage) -> plain digits.
pub fn normalize_load_value(text: &str) -> String {
    let value = text.trim();
    if ZERO_CENTS.is_match(value) {
        return value[..value.len() - 3].to_owned();
    }
    if let Some(caps) = SCIENTIFIC.captures(value) {
        let sign = caps.get(1).map_or("", |m| m.as_str());
        let int = caps.get(2).map_or("", |m| m.as_str());
        let frac = caps.get(3).map_or("", |m| m.as_str());
        let exponent = caps.get(4).map_or("", |m| m.as_str());
        if let Ok(exponent) = exponent.parse::<i64>() {
            return expand_scientific(sign, int, frac, exponent);
        }
    }
    value.to_owned()
}

/// Exact plain-digit expansion rounded to 20 decimals (Go
/// `big.Rat.FloatString(20)` with trailing zeros trimmed).
fn expand_scientific(sign: &str, int: &str, frac: &str, exponent: i64) -> String {
    const MAX_DECIMALS: usize = 20;
    let digits = format!("{int}{frac}");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return "0".to_owned();
    }
    // Position of the decimal point inside `digits` once the exponent is applied.
    let leading_zeros = (int.len() + frac.len()) - digits.len();
    let point = int.len() as i64 + exponent - leading_zeros as i64;
    let (mut whole, mut fraction) = if point <= 0 {
        let zeros = usize::try_from(-point).unwrap_or(usize::MAX).min(4096);
        ("0".to_owned(), format!("{}{}", "0".repeat(zeros), digits))
    } else if point as usize >= digits.len() {
        let zeros = (point as usize - digits.len()).min(4096);
        (format!("{digits}{}", "0".repeat(zeros)), String::new())
    } else {
        let split = point as usize;
        (digits[..split].to_owned(), digits[split..].to_owned())
    };
    if fraction.len() > MAX_DECIMALS {
        let round_up = fraction.as_bytes()[MAX_DECIMALS] >= b'5';
        fraction.truncate(MAX_DECIMALS);
        if round_up {
            let combined = increment_decimal(&format!("{whole}{fraction}"));
            let split = combined.len() - MAX_DECIMALS;
            whole = combined[..split].to_owned();
            fraction = combined[split..].to_owned();
        }
    }
    let fraction = fraction.trim_end_matches('0');
    let body = if fraction.is_empty() {
        whole
    } else {
        format!("{whole}.{fraction}")
    };
    if body == "0" {
        body
    } else {
        format!("{}{body}", if sign == "-" { "-" } else { "" })
    }
}

/// Adds one unit in the last place of a digit string (keeps the length unless
/// it overflows to a new digit).
fn increment_decimal(digits: &str) -> String {
    let mut out: Vec<u8> = digits.bytes().collect();
    let mut i = out.len();
    loop {
        if i == 0 {
            out.insert(0, b'1');
            break;
        }
        i -= 1;
        if out[i] == b'9' {
            out[i] = b'0';
        } else {
            out[i] += 1;
            break;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: [&str; 61] = [
        "nit",
        "plan",
        "subplan",
        "factura",
        "factura mc",
        "factura consolidada",
        "tipo de identificacion",
        "no. documento",
        "beneficiario",
        "categoria actual",
        "pto. venta",
        "no. formula",
        "no. ssc",
        "tipo formula",
        "fecha",
        "lote",
        "plu",
        "articulo",
        "cantidad",
        "vlr. unitario",
        "costo unitario",
        "vlr. total",
        "cuota",
        "margen",
        "estado",
        "usuario despacha",
        "nombre usuario despacha",
        "numero autorizacion",
        "mipres",
        "observaciones",
        "fecha de descarga",
        "fecha elaboracion factura",
        "usuario elabora",
        "nombre usuario elabora",
        "cum",
        "numero entregas",
        "cantidad diaria",
        "fecha proxima entrega",
        "autorizacion medicamento",
        "factura homologo",
        "cod. homologo",
        "descripcion homologo",
        "cantidad homologo",
        "vlr. unitario homologo",
        "modalidad",
        "fecha formula",
        "fecha autorizacion",
        "codigo diagnostico",
        "nombre diagnostico",
        "domicilio",
        "direccion entrega",
        "telefono entrega",
        "causal exento",
        "ips",
        "id medico",
        "nombre medico",
        "porcentaje iva",
        "genero paciente",
        "fecha nacimiento",
        "departamento",
        "municipio",
    ];

    fn row(sep: &str, values: &[(&str, &str)]) -> String {
        HEADER
            .iter()
            .map(|h| values.iter().find(|(k, _)| k == h).map_or("", |(_, v)| *v))
            .collect::<Vec<_>>()
            .join(sep)
    }

    fn synthetic<'a>(
        invoice: &'a str,
        plu: &'a str,
        cum: &'a str,
        qty: &'a str,
        unit: &'a str,
        total: &'a str,
    ) -> Vec<(&'a str, &'a str)> {
        vec![
            ("nit", "900000001"),
            ("factura", invoice),
            ("tipo de identificacion", "CC"),
            ("no. documento", "1000000001"),
            ("beneficiario", "PACIENTE PRUEBA"),
            ("fecha", "2026-09-01 10:00:00"),
            ("plu", plu),
            ("articulo", "PRODUCTO PRUEBA"),
            ("cantidad", qty),
            ("vlr. unitario", unit),
            ("vlr. total", total),
            ("cuota", "0"),
            ("cum", cum),
            ("causal exento", "Regimen Subsidiado"),
            ("genero paciente", "F"),
        ]
    }

    fn read(text: &str) -> Result<ReadResult> {
        read_csv(text.as_bytes())
    }

    #[test]
    fn columns_are_matched_by_normalized_header_name_with_bom_and_crlf() {
        let mut header: Vec<String> = HEADER.iter().map(|h| (*h).to_owned()).collect();
        header[3] = "Factura MC".into();
        header[4] = " FACTURA ".into();
        header[6] = "Tipo de Identificación".into();
        let mut values = synthetic(
            "MC-1",
            "20010_01_123",
            "19950404-1",
            "2",
            "1.500,00",
            "3000,00",
        );
        values.push(("factura mc", "EVS-100"));
        let text = format!(
            "\u{feff}{}\r\n{}\r\n\r\n",
            header.join("|"),
            row("|", &values)
        );
        let result = read(&text).expect("csv");
        assert_eq!((result.lines.len(), result.skipped), (1, 0));
        let line = &result.lines[0];
        // Columns 3 and 4 were swapped in the header: "factura" now holds the 5th field.
        assert_eq!(line.invoice, "EVS100");
        assert_eq!(line.invoice_raw, "EVS-100");
        assert_eq!(line.doc_type, "CC");
        assert_eq!(line.quantity, "2");
        assert_eq!(line.total_value, "3000", "123,00 loses its zero cents");
        assert_eq!(line.unit_value, "1.500,00", "only ',00' is stripped");
        assert_eq!(line.source_line, 2);
        assert_eq!(line.causal_exento, "Regimen Subsidiado");
        assert_eq!(line.gender, "F");
        assert_eq!(result.separator, '|');
    }

    #[test]
    fn semicolon_files_and_scientific_notation() {
        let values = synthetic(
            "EVS200",
            "20010_01_123",
            "1.9950404E7",
            "1",
            "100,00",
            " 100,00 ",
        );
        let text = format!("{}\n{}\n", HEADER.join(";"), row(";", &values));
        let result = read(&text).expect("csv");
        let line = &result.lines[0];
        assert_eq!(line.cum, "19950404");
        assert_eq!(line.agreement_cum, "19950404");
        assert_eq!(line.total_value, "100");
        assert_eq!(line.unit_value, "100");
        assert_eq!(result.separator, ';');
    }

    #[test]
    fn rows_with_a_wrong_field_count_are_skipped_with_their_line_numbers() {
        let good = row("|", &synthetic("EVS1", "1_1", "1", "1", "1", "1"));
        let text = format!(
            "{}\n{good}\na|b|c\n{good}|extra\nno separator\n",
            HEADER.join("|")
        );
        let result = read(&text).expect("csv");
        assert_eq!(result.lines.len(), 1);
        assert_eq!(result.skipped, 3);
        assert_eq!(result.skipped_lines, vec![3, 4, 5]);
    }

    #[test]
    fn required_columns_and_empty_files_are_errors() {
        let err = read("nit|plan|factura\n1|2|3\n").expect_err("missing columns");
        let message = err.to_string();
        assert!(message.starts_with("convenio: missing required columns: "));
        assert!(message.contains("vlr. total") && message.contains("cum"));
        assert_eq!(
            read("").expect_err("empty").to_string(),
            "convenio: the CSV is empty"
        );
        assert_eq!(
            read("just one column\n")
                .expect_err("no separator")
                .to_string(),
            "convenio: the header has no '|' or ';' separator"
        );
    }

    #[test]
    fn header_normalization() {
        assert_eq!(
            normalize_header(" Tipo de  Identificación "),
            "tipo de identificacion"
        );
        assert_eq!(normalize_header("VLR. TOTAL"), "vlr. total");
        assert_eq!(normalize_header("Género Paciente"), "genero paciente");
    }

    #[test]
    fn load_value_normalization() {
        for (input, expected) in [
            ("100,00", "100"),
            ("-5,00", "-5"),
            ("1,50", "1,50"),
            ("1.5E3", "1500"),
            ("2,5e-1", "0.25"),
            (" x ", "x"),
            ("ABC", "ABC"),
            ("0E5", "0"),
            ("-1.5e2", "-150"),
            ("1e-25", "0"),
            ("123456789012345678901234E2", "12345678901234567890123400"),
        ] {
            assert_eq!(normalize_load_value(input), expected, "{input}");
        }
        // Rounded half up at the 20th decimal, like big.Rat.FloatString(20).
        assert_eq!(normalize_load_value("5e-21"), "0.00000000000000000001");
        assert_eq!(normalize_load_value("4e-21"), "0");
    }

    #[test]
    fn prepared_format_is_detected_from_any_line() {
        let mut header: Vec<String> = HEADER.iter().map(|h| (*h).to_owned()).collect();
        header.push("CLASIFICACION".into());
        header.push("MED-FORMA FARMACEUTICA RIPS".into());
        header.push("MED-CONCENTRACION MEDICAMENTO RIPS".into());
        header.push("MED-UNIDAD MEDIDA RIPS".into());
        let base = row(
            "|",
            &synthetic("EVS400", "20010_01_123", "19950404-1", "1", "100", "100"),
        );
        for (class, expected) in [("714", true), ("999", false), (" 799 ", true)] {
            let text = format!("{}\n{base}|{class}|TABLETA|500|MG\n", header.join("|"));
            let result = read(&text).expect("csv");
            assert_eq!(result.prepared_format, expected, "{class}");
            assert_eq!(result.lines[0].pharmaceutical_form, "TABLETA");
            assert_eq!(result.lines[0].concentration, "500");
            assert_eq!(result.lines[0].measure_unit, "MG");
        }
        let automatic = format!("{}\n{base}\n", HEADER.join("|"));
        assert!(!read(&automatic).expect("csv").prepared_format);
    }

    #[test]
    fn json_shape_uses_the_go_tags_and_omits_empty_values() {
        let line = Line {
            invoice: "EVS1".into(),
            plu: "1_1".into(),
            ..Line::default()
        };
        let json = serde_json::to_value(&line).expect("json");
        assert_eq!(json["factura"], "EVS1");
        assert!(json.get("nit").is_none(), "omitempty");
        assert!(json.get("cuota").is_some(), "required fields stay");
        let back: Line = serde_json::from_value(json).expect("back");
        assert_eq!(back, line);
    }
}
