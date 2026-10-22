//! Prepared-format loads: rights come from the file's own (repurposed)
//! columns, with no validator, token or cache.
//!
//! Port of Go `internal/derechos/file.go`. These files carry the affiliate's
//! régimen in `Causal exento`, the DANE code in `Departamento`, the zona in
//! `Municipio`, plus `Genero Paciente` and `Fecha Nacimiento`. A blank cell is
//! filled from the other lines of the invoice; two different normalized
//! values for a field put the invoice in manual review.

use std::collections::HashSet;

use chrono::NaiveDate;

use super::availability::Stats;
use super::error::DerechosError;
use super::rights::{REASON_INCOMPLETE, REASON_NO_DOCUMENT, Rights};
use super::service::{
    AffiliateRights, InvoiceInput, InvoiceResult, LineInput, SOURCE_FILE, distinct_affiliates,
};

/// Reason prefix of an invoice whose lines disagree on a rights field.
pub const REASON_INCONSISTENT: &str =
    "Datos de derechos inconsistentes entre las lineas de la factura (archivo): ";

/// Régimen code of `Causal exento`: `EPS048 -> 01`, `EPSS48 -> 04`,
/// `ESS207 -> 04`, `ESSC07 -> 01`, the words `CONTRIBUTIVO`/`SUBSIDIADO`, or an
/// already converted `01`/`04`. Anything else is empty (régimen missing).
pub fn normalize_user_type(text: &str) -> String {
    match text.trim().to_uppercase().as_str() {
        "EPS048" | "ESSC07" | "CONTRIBUTIVO" | "01" => "01",
        "EPSS48" | "ESS207" | "SUBSIDIADO" | "04" => "04",
        _ => "",
    }
    .to_owned()
}

/// Zona: `U`/`R`, the RIPS codes `02` (urbana) / `01` (rural) or a text
/// containing `URBAN`/`RURAL` (checked in this order, so a text with both is
/// rural). Anything else is empty (zona missing).
pub fn normalize_zone(text: &str) -> String {
    let zone = text.trim().to_uppercase();
    let normalized = if zone == "01" || zone.contains("RURAL") {
        "R"
    } else if zone == "02" || zone.contains("URBAN") {
        "U"
    } else if zone == "R" || zone == "U" {
        zone.as_str()
    } else {
        ""
    };
    normalized.to_owned()
}

/// Sexo: `M`, `F`, `I`, `MASCULINO`, `FEMENINO`. Anything else is empty (the
/// legacy passed it through; here it counts as missing).
pub fn normalize_sex(text: &str) -> String {
    match text.trim().to_uppercase().as_str() {
        "M" | "MASCULINO" => "M",
        "F" | "FEMENINO" => "F",
        "I" => "I",
        _ => "",
    }
    .to_owned()
}

/// Birth date as `yyyy-MM-dd`: `yyyy-MM-dd` (extra time text ignored),
/// `d/MM/yyyy`, `dd-MM-yyyy` or `yyyy/MM/dd`. Unparseable is empty (missing).
pub fn normalize_birth_date(text: &str) -> String {
    let text = text.trim();
    let bytes = text.as_bytes();
    if bytes.len() >= 10 && bytes[4] == b'-' && bytes[7] == b'-' {
        return text
            .get(..10)
            .and_then(|head| parse_parts(head, '-', [4, 2, 2]))
            .map_or_else(String::new, format_date);
    }
    let Some(day) = text.split_whitespace().next() else {
        return String::new();
    };
    parse_day_first(day, '/')
        .or_else(|| parse_day_first(day, '-'))
        .or_else(|| parse_parts(day, '/', [4, 2, 2]))
        .map_or_else(String::new, format_date)
}

fn format_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// `d/MM/yyyy` or `dd/MM/yyyy` (same with another separator).
fn parse_day_first(text: &str, separator: char) -> Option<NaiveDate> {
    let mut parts = text.split(separator);
    let (day, month, year) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || day.is_empty() || day.len() > 2 {
        return None;
    }
    let padded = format!("{day:0>2}");
    parse_parts(
        &format!("{padded}{separator}{month}{separator}{year}"),
        separator,
        [2, 2, 4],
    )
}

/// Splits `text` on `separator` into three all-digit parts of the exact
/// widths, read as year-month-day for widths `[4, 2, 2]` and as
/// day-month-year for `[2, 2, 4]`.
fn parse_parts(text: &str, separator: char, widths: [usize; 3]) -> Option<NaiveDate> {
    let parts: Vec<&str> = text.split(separator).collect();
    if parts.len() != 3 {
        return None;
    }
    let mut numbers = [0u32; 3];
    for (index, part) in parts.iter().enumerate() {
        if part.len() != widths[index] || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        numbers[index] = part.parse().ok()?;
    }
    if widths[0] == 4 {
        NaiveDate::from_ymd_opt(i32::try_from(numbers[0]).ok()?, numbers[1], numbers[2])
    } else {
        NaiveDate::from_ymd_opt(i32::try_from(numbers[2]).ok()?, numbers[1], numbers[0])
    }
}

/// One rights field read from every line of an invoice.
struct FileField {
    /// Spanish label, the same as [`Rights::missing`].
    label: &'static str,
    raw: fn(&LineInput) -> &str,
    normalize: fn(&str) -> String,
}

const FILE_FIELDS: [FileField; 5] = [
    FileField {
        label: "regimen",
        raw: |l| &l.causal_exento,
        normalize: normalize_user_type,
    },
    FileField {
        label: "zona",
        raw: |l| &l.municipio,
        normalize: normalize_zone,
    },
    FileField {
        label: "municipio DANE",
        raw: |l| &l.departamento,
        normalize: |s| s.trim().to_owned(),
    },
    FileField {
        label: "sexo",
        raw: |l| &l.csv_gender,
        normalize: normalize_sex,
    },
    FileField {
        label: "fecha de nacimiento",
        raw: |l| &l.csv_birth_date,
        normalize: normalize_birth_date,
    },
];

impl FileField {
    /// The normalized value of the first filled cell (empty when blank or not
    /// interpretable) and whether the lines disagree. A filled cell that is
    /// not interpretable counts as its own distinct value, so "valid vs
    /// garbage" is a disagreement, not a silent pick.
    fn value(&self, lines: &[LineInput]) -> (String, bool) {
        let mut first = String::new();
        let mut seen = HashSet::new();
        for line in lines {
            let raw = (self.raw)(line).trim();
            if raw.is_empty() {
                continue;
            }
            let value = (self.normalize)(raw);
            let key = if value.is_empty() {
                format!("?{}", raw.to_uppercase())
            } else {
                value.clone()
            };
            if seen.is_empty() {
                first = value;
            }
            seen.insert(key);
        }
        (first, seen.len() > 1)
    }
}

/// The rights of an invoice from its lines' own columns, and the labels of
/// the fields whose lines disagree.
pub fn rights_from_file(input: &InvoiceInput) -> (Rights, Vec<&'static str>) {
    let mut values = Vec::with_capacity(FILE_FIELDS.len());
    let mut conflicts = Vec::new();
    for field in &FILE_FIELDS {
        let (value, conflict) = field.value(&input.lines);
        values.push(value);
        if conflict {
            conflicts.push(field.label);
        }
    }
    let mut values = values.into_iter();
    let mut next = || values.next().unwrap_or_default();
    let rights = Rights {
        user_type: next(),
        zone: next(),
        dane: next(),
        sex: next(),
        birth_date: next(),
    };
    (rights, conflicts)
}

/// Classifies prepared-format invoices without querying anything: `DONE`
/// (source `archivo`) when the file carries complete rights, otherwise
/// `REVISION_MANUAL` (no document, inconsistent lines, or incomplete data).
/// Like the validator path it fails for an invoice naming several affiliates.
/// The stats count the distinct affiliates; none is queried, so the
/// validator-down alert never applies.
pub fn from_file(invoices: &[InvoiceInput]) -> Result<(Vec<InvoiceResult>, Stats), DerechosError> {
    let owners = invoices
        .iter()
        .map(InvoiceInput::affiliate)
        .collect::<Result<Vec<_>, _>>()?;
    let mut results = Vec::with_capacity(invoices.len());
    for (input, owner) in invoices.iter().zip(&owners) {
        if !owner.has_document() {
            results.push(InvoiceResult::manual_review(
                &input.invoice,
                REASON_NO_DOCUMENT.to_owned(),
            ));
            continue;
        }
        let (rights, conflicts) = rights_from_file(input);
        let missing = rights.missing();
        results.push(if !conflicts.is_empty() {
            InvoiceResult::manual_review(
                &input.invoice,
                format!("{REASON_INCONSISTENT}{}", conflicts.join(", ")),
            )
        } else if !missing.is_empty() {
            InvoiceResult::manual_review(
                &input.invoice,
                format!("{REASON_INCOMPLETE}{}", missing.join(", ")),
            )
        } else {
            InvoiceResult::done(
                &input.invoice,
                AffiliateRights {
                    affiliate: owner.clone(),
                    rights,
                    source: SOURCE_FILE,
                    queried_doc_type: None,
                },
            )
        });
    }
    let stats = Stats {
        affiliates: distinct_affiliates(&owners).len(),
        ..Stats::default()
    };
    Ok((results, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::PhaseState;

    fn line(regimen: &str, dane: &str, zona: &str, sexo: &str, fecha: &str) -> LineInput {
        LineInput {
            doc_type: "CC".into(),
            doc_number: "10".into(),
            csv_gender: sexo.into(),
            csv_birth_date: fecha.into(),
            causal_exento: regimen.into(),
            departamento: dane.into(),
            municipio: zona.into(),
        }
    }

    fn invoice(name: &str, lines: Vec<LineInput>) -> InvoiceInput {
        InvoiceInput {
            invoice: name.into(),
            lines,
        }
    }

    #[test]
    fn user_type_table() {
        for (raw, want) in [
            ("EPS048", "01"),
            (" epss48 ", "04"),
            ("ESS207", "04"),
            ("ESSC07", "01"),
            ("contributivo", "01"),
            ("Subsidiado", "04"),
            ("01", "01"),
            ("04", "04"),
            ("02", ""),
            ("EPS04", ""),
            ("", ""),
        ] {
            assert_eq!(normalize_user_type(raw), want, "{raw:?}");
        }
    }

    #[test]
    fn zone_table() {
        for (raw, want) in [
            ("U", "U"),
            ("r", "R"),
            ("02", "U"),
            ("01", "R"),
            ("zona urbana", "U"),
            ("RURAL DISPERSO", "R"),
            ("URBANO Y RURAL", "R"),
            ("03", ""),
            ("X", ""),
        ] {
            assert_eq!(normalize_zone(raw), want, "{raw:?}");
        }
    }

    #[test]
    fn sex_table() {
        for (raw, want) in [
            ("m", "M"),
            ("MASCULINO", "M"),
            ("f", "F"),
            (" Femenino ", "F"),
            ("I", "I"),
            ("HOMBRE", ""),
            ("", ""),
        ] {
            assert_eq!(normalize_sex(raw), want, "{raw:?}");
        }
    }

    #[test]
    fn birth_date_formats() {
        for (raw, want) in [
            ("1990-01-31", "1990-01-31"),
            ("1990-01-31 00:00:00", "1990-01-31"),
            ("1990-01-31T10:00:00", "1990-01-31"),
            ("5/03/2001", "2001-03-05"),
            ("05/03/2001", "2001-03-05"),
            ("05/03/2001 12:30", "2001-03-05"),
            ("05-03-2001", "2001-03-05"),
            ("5-03-2001", "2001-03-05"),
            ("2001/03/05", "2001-03-05"),
            ("2001-02-30", ""),
            ("31/02/2001", ""),
            ("5/3/2001", ""),
            ("05.03.2001", ""),
            ("garbage", ""),
            ("", ""),
        ] {
            assert_eq!(normalize_birth_date(raw), want, "{raw:?}");
        }
    }

    #[test]
    fn complete_rights_are_done_with_source_archivo() {
        let invoices = [invoice(
            "F1",
            vec![line("ESS207", "13001", "02", "FEMENINO", "05/03/2001")],
        )];
        let (results, stats) = from_file(&invoices).expect("from file");
        let result = &results[0];
        assert_eq!(result.state, PhaseState::Done);
        assert!(result.reason.is_empty());
        let affiliate = result.affiliate.as_ref().expect("rights");
        assert_eq!(affiliate.source, "archivo");
        assert_eq!(
            affiliate.rights,
            Rights {
                user_type: "04".into(),
                zone: "U".into(),
                dane: "13001".into(),
                sex: "F".into(),
                birth_date: "2001-03-05".into(),
            }
        );
        assert_eq!(
            stats,
            Stats {
                affiliates: 1,
                ..Stats::default()
            }
        );
    }

    #[test]
    fn blank_cells_are_filled_from_the_other_lines() {
        let invoices = [invoice(
            "F1",
            vec![
                line("", "13001", "", "M", ""),
                line("EPS048", "", "R", "", "2001-03-05"),
                line("01", "13001", "01", "MASCULINO", "05/03/2001"),
            ],
        )];
        let (results, _) = from_file(&invoices).expect("from file");
        assert_eq!(results[0].state, PhaseState::Done);
        let rights = &results[0].affiliate.as_ref().expect("rights").rights;
        assert_eq!(rights.user_type, "01");
        assert_eq!(rights.zone, "R");
        assert_eq!(rights.sex, "M");
        assert_eq!(rights.birth_date, "2001-03-05");
    }

    #[test]
    fn different_normalized_values_are_inconsistent_and_equal_ones_are_not() {
        let conflicting = invoice(
            "F1",
            vec![
                line("EPS048", "13001", "U", "M", "2001-03-05"),
                line("EPSS48", "13001", "U", "F", "2001-03-05"),
            ],
        );
        let (results, _) = from_file(&[conflicting]).expect("from file");
        assert_eq!(results[0].state, PhaseState::RevisionManual);
        assert_eq!(
            results[0].reason,
            "Datos de derechos inconsistentes entre las lineas de la factura (archivo): regimen, sexo"
        );
        assert!(results[0].affiliate.is_none());
    }

    #[test]
    fn valid_versus_garbage_is_a_disagreement() {
        let invoices = [invoice(
            "F1",
            vec![
                line("EPS048", "13001", "U", "M", "2001-03-05"),
                line("EPS048", "13001", "U", "M", "garbage"),
            ],
        )];
        let (results, _) = from_file(&invoices).expect("from file");
        assert_eq!(
            results[0].reason,
            "Datos de derechos inconsistentes entre las lineas de la factura (archivo): fecha de nacimiento"
        );
    }

    #[test]
    fn incomplete_rights_list_the_missing_fields() {
        let invoices = [invoice("F1", vec![line("XX", "", "U", "HOMBRE", "")])];
        let (results, _) = from_file(&invoices).expect("from file");
        assert_eq!(results[0].state, PhaseState::RevisionManual);
        assert_eq!(
            results[0].reason,
            "Datos de derechos incompletos: faltan regimen, municipio DANE, sexo, fecha de nacimiento"
        );
    }

    #[test]
    fn invoices_without_a_document_and_with_two_affiliates() {
        let mut no_doc = line("EPS048", "13001", "U", "M", "2001-03-05");
        no_doc.doc_number = " ".into();
        let (results, stats) = from_file(&[invoice("F1", vec![no_doc])]).expect("from file");
        assert_eq!(
            results[0].reason,
            "Factura sin tipo o numero de documento del afiliado"
        );
        assert_eq!(stats.affiliates, 0);

        let mut other = line("EPS048", "13001", "U", "M", "2001-03-05");
        other.doc_number = "11".into();
        let two = invoice(
            "F2",
            vec![line("EPS048", "13001", "U", "M", "2001-03-05"), other],
        );
        let err = from_file(&[two]).expect_err("two affiliates");
        assert!(matches!(err, DerechosError::MultipleAffiliates { .. }));
    }
}
