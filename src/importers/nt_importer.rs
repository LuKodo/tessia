//! Reads the Nota Técnica workbook and turns its rows into validated
//! [`NewNtRecord`]s. Knows nothing about persistence.

use std::fmt;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use calamine::{Data, Range, Reader, Xlsx, XlsxError, open_workbook};
use thiserror::Error;

use crate::config::{NtColumnMapping, NtImportConfig};
use crate::models::nt_record::NewNtRecord;

#[derive(Debug, Error)]
pub enum NtImportError {
    #[error("No fue posible abrir la Nota Técnica: {}", .path.display())]
    Open {
        path: PathBuf,
        #[source]
        source: XlsxError,
    },
    #[error("La Nota Técnica no contiene hojas")]
    NoSheets,
    #[error("No se encontró la hoja '{0}' en la Nota Técnica")]
    SheetNotFound(String),
    #[error("No fue posible leer la hoja '{sheet}' de la Nota Técnica")]
    ReadSheet {
        sheet: String,
        #[source]
        source: XlsxError,
    },
    #[error("No se encontró la fila de encabezados (fila {0}) o está vacía")]
    HeaderRowNotFound(u32),
    #[error("{}", missing_columns_message(.0))]
    MissingColumns(Vec<String>),
    #[error("Fila {0}: el número de fila excede el máximo soportado")]
    RowOutOfRange(u32),
}

fn missing_columns_message(headers: &[String]) -> String {
    match headers {
        [single] => format!("No se encontró la columna {single}"),
        many => format!("No se encontraron las columnas {}", many.join(", ")),
    }
}

/// The persisted columns of a Nota Técnica row, in storage order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NtField {
    Cum,
    CodGenerico,
    FormaFarmaceutica,
    Concentracion,
    UnidadMedida,
    Clasificacion,
    Precio,
}

const FIELD_COUNT: usize = 7;

impl NtField {
    const ALL: [NtField; FIELD_COUNT] = [
        NtField::Cum,
        NtField::CodGenerico,
        NtField::FormaFarmaceutica,
        NtField::Concentracion,
        NtField::UnidadMedida,
        NtField::Clasificacion,
        NtField::Precio,
    ];

    /// Human-facing field name used in error messages.
    fn label(self) -> &'static str {
        match self {
            NtField::Cum => "CUM",
            NtField::CodGenerico => "COD_GENERICO",
            NtField::FormaFarmaceutica => "FORMA_FARMACEUTICA",
            NtField::Concentracion => "CONCENTRACION",
            NtField::UnidadMedida => "UNIDAD_MEDIDA",
            NtField::Clasificacion => "CLASIFICACION",
            NtField::Precio => "PRECIO",
        }
    }

    fn expected_header(self, mapping: &NtColumnMapping) -> &str {
        match self {
            NtField::Cum => &mapping.cum,
            NtField::CodGenerico => &mapping.cod_generico,
            NtField::FormaFarmaceutica => &mapping.forma_farmaceutica,
            NtField::Concentracion => &mapping.concentracion,
            NtField::UnidadMedida => &mapping.unidad_medida,
            NtField::Clasificacion => &mapping.clasificacion,
            NtField::Precio => &mapping.precio,
        }
    }
}

/// A non-empty row that was not imported because required fields are empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedRow {
    /// Absolute 1-based Excel row.
    pub row_number: i32,
    /// Labels of every empty required field, in column order.
    pub empty_fields: Vec<&'static str>,
}

impl fmt::Display for SkippedRow {
    /// `Fila 1735: campo PRECIO vacío` or `Fila 12: campos CUM, PRECIO vacíos`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fields = self.empty_fields.join(", ");
        match self.empty_fields.len() {
            1 => write!(f, "Fila {}: campo {fields} vacío", self.row_number),
            _ => write!(f, "Fila {}: campos {fields} vacíos", self.row_number),
        }
    }
}

/// Outcome of reading a Nota Técnica: rows ready to store plus rows left out.
///
/// Fully empty rows appear in neither list.
#[derive(Debug)]
pub struct NtImportResult {
    pub records: Vec<NewNtRecord>,
    pub skipped: Vec<SkippedRow>,
}

/// Reads the Nota Técnica at `path`.
pub fn import_nt(path: &Path, config: &NtImportConfig) -> Result<NtImportResult, NtImportError> {
    let mut workbook: Xlsx<BufReader<File>> =
        open_workbook(path).map_err(|source| NtImportError::Open {
            path: path.to_path_buf(),
            source,
        })?;
    let sheet = select_sheet(&workbook.sheet_names(), config.sheet.as_deref())?;
    let range = workbook
        .worksheet_range(&sheet)
        .map_err(|source| NtImportError::ReadSheet { sheet, source })?;
    map_range(&range, config)
}

fn select_sheet(names: &[String], requested: Option<&str>) -> Result<String, NtImportError> {
    match requested {
        Some(name) => names
            .iter()
            .find(|candidate| candidate.as_str() == name)
            .cloned()
            .ok_or_else(|| NtImportError::SheetNotFound(name.to_owned())),
        None => names.first().cloned().ok_or(NtImportError::NoSheets),
    }
}

/// Maps the rows below the header row of `range` into records and skipped rows.
///
/// Row numbers are absolute Excel rows (1-based), even when the used range
/// does not start at A1. Columns are resolved by header text, not position.
fn map_range(
    range: &Range<Data>,
    config: &NtImportConfig,
) -> Result<NtImportResult, NtImportError> {
    let header_not_found = || NtImportError::HeaderRowNotFound(config.header_row);
    let (first_row, _) = range.start().ok_or_else(header_not_found)?;
    // Absolute 0-based row index of the header row, then relative to the range.
    let header_index = config
        .header_row
        .checked_sub(1)
        .ok_or_else(header_not_found)?;
    let header_offset = header_index
        .checked_sub(first_row)
        .ok_or_else(header_not_found)?;
    let header_offset = usize::try_from(header_offset).map_err(|_| header_not_found())?;

    let mut rows = range.rows().enumerate().skip(header_offset);
    let (_, header) = rows.next().ok_or_else(header_not_found)?;
    if header.iter().all(is_blank) {
        return Err(header_not_found());
    }
    let columns = resolve_columns(header, &config.mapping)?;

    let mut result = NtImportResult {
        records: Vec::new(),
        skipped: Vec::new(),
    };
    for (offset, row) in rows {
        let excel_row = excel_row_number(first_row, offset);
        let values =
            columns.map(|position| row.get(position).map(cell_to_string).unwrap_or_default());
        let empty_fields = empty_fields(&values);
        if empty_fields.len() == FIELD_COUNT {
            tracing::debug!(row = excel_row, "ignoring empty row");
            continue;
        }
        let row_number =
            i32::try_from(excel_row).map_err(|_| NtImportError::RowOutOfRange(excel_row))?;
        if empty_fields.is_empty() {
            result.records.push(build_record(row_number, values));
        } else {
            result.skipped.push(SkippedRow {
                row_number,
                empty_fields,
            });
        }
    }
    Ok(result)
}

fn excel_row_number(first_row: u32, offset: usize) -> u32 {
    // Saturates instead of overflowing; `map_range` rejects anything that
    // does not fit the stored `i32`.
    u32::try_from(offset).map_or(u32::MAX, |offset| {
        first_row.saturating_add(offset).saturating_add(1)
    })
}

fn is_blank(cell: &Data) -> bool {
    cell_to_string(cell).is_empty()
}

/// Finds the position of each field's column in the header row, in
/// [`NtField::ALL`] order. Fails naming every expected header that is missing.
fn resolve_columns(
    header: &[Data],
    mapping: &NtColumnMapping,
) -> Result<[usize; FIELD_COUNT], NtImportError> {
    let titles: Vec<String> = header.iter().map(cell_to_string).collect();
    let mut positions = [0; FIELD_COUNT];
    let mut missing = Vec::new();
    for (slot, field) in positions.iter_mut().zip(NtField::ALL) {
        let expected = field.expected_header(mapping);
        match titles
            .iter()
            .position(|title| header_matches(title, expected))
        {
            Some(position) => *slot = position,
            None => missing.push(expected.trim().to_owned()),
        }
    }
    if missing.is_empty() {
        Ok(positions)
    } else {
        Err(NtImportError::MissingColumns(missing))
    }
}

/// Header matching rule: trimmed and case-insensitive.
fn header_matches(cell_text: &str, expected: &str) -> bool {
    cell_text.trim().to_lowercase() == expected.trim().to_lowercase()
}

/// Domain rule for field values: every field is required (non-empty after trim).
fn is_valid_value(value: &str) -> bool {
    !value.trim().is_empty()
}

/// Labels of the required fields whose value breaks the domain rule, in column order.
fn empty_fields(values: &[String; FIELD_COUNT]) -> Vec<&'static str> {
    NtField::ALL
        .iter()
        .zip(values)
        .filter(|(_, value)| !is_valid_value(value))
        .map(|(field, _)| field.label())
        .collect()
}

fn build_record(row_number: i32, values: [String; FIELD_COUNT]) -> NewNtRecord {
    let [
        cum,
        cod_generico,
        forma_farmaceutica,
        concentracion,
        unidad_medida,
        clasificacion,
        precio,
    ] = values;
    NewNtRecord {
        row_number,
        cum,
        cod_generico,
        forma_farmaceutica,
        concentracion,
        unidad_medida,
        clasificacion,
        precio,
    }
}

/// Converts a cell to its text form.
///
/// - strings are trimmed;
/// - integers use decimal notation;
/// - finite floats without a fractional part drop the ".0" (so a numeric CUM
///   stored as `123.0` becomes `123`);
/// - other floats use their plain text (no locale rules: the price format is
///   not interpreted);
/// - booleans, dates and errors use their display text;
/// - empty cells become an empty string.
fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::String(text) => text.trim().to_owned(),
        Data::Float(value) if value.is_finite() && value.fract() == 0.0 => format!("{value:.0}"),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(start: (u32, u32), rows: &[&[&str]]) -> Range<Data> {
        let width = rows.iter().map(|row| row.len()).max().unwrap_or(1) as u32;
        let end = (start.0 + rows.len() as u32 - 1, start.1 + width - 1);
        let mut range = Range::new(start, end);
        for (r, row) in rows.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                if !text.is_empty() {
                    range.set_value(
                        (start.0 + r as u32, start.1 + c as u32),
                        Data::String((*text).to_owned()),
                    );
                }
            }
        }
        range
    }

    const HEADER: [&str; 7] = [
        "CUM",
        "COD_GENERICO",
        "FORMA_FARMACEUTICA",
        "CONCENTRACION",
        "UNIDAD_MEDIDA",
        "CLASIFICACION",
        "PRECIO",
    ];

    /// Mapping onto `HEADER`; independent of the real-file defaults in `config`.
    fn test_config() -> NtImportConfig {
        NtImportConfig {
            sheet: None,
            header_row: 1,
            mapping: NtColumnMapping {
                cum: "CUM".into(),
                cod_generico: "COD_GENERICO".into(),
                forma_farmaceutica: "FORMA_FARMACEUTICA".into(),
                concentracion: "CONCENTRACION".into(),
                unidad_medida: "UNIDAD_MEDIDA".into(),
                clasificacion: "CLASIFICACION".into(),
                precio: "PRECIO".into(),
            },
        }
    }

    fn full_row(cum: &str) -> [&str; 7] {
        [cum, "G1", "TABLETA", "500 MG", "UN", "POS", "100,5"]
    }

    #[test]
    fn cell_to_string_trims_strings() {
        assert_eq!(cell_to_string(&Data::String("  abc \n".into())), "abc");
    }

    #[test]
    fn cell_to_string_formats_integers() {
        assert_eq!(cell_to_string(&Data::Int(-42)), "-42");
    }

    #[test]
    fn cell_to_string_drops_zero_fraction_from_floats() {
        assert_eq!(cell_to_string(&Data::Float(123.0)), "123");
        assert_eq!(
            cell_to_string(&Data::Float(20_001_234_567.0)),
            "20001234567"
        );
    }

    #[test]
    fn cell_to_string_keeps_fractional_floats() {
        assert_eq!(cell_to_string(&Data::Float(1234.5)), "1234.5");
    }

    #[test]
    fn cell_to_string_handles_non_finite_floats() {
        assert_eq!(cell_to_string(&Data::Float(f64::INFINITY)), "inf");
    }

    #[test]
    fn cell_to_string_formats_bool_and_empty() {
        assert_eq!(cell_to_string(&Data::Bool(true)), "true");
        assert_eq!(cell_to_string(&Data::Empty), "");
    }

    #[test]
    fn header_matching_ignores_case_and_surrounding_spaces() {
        assert!(header_matches("  cum ", "CUM"));
        assert!(header_matches("Forma_Farmacéutica", "forma_farmacéutica"));
        assert!(!header_matches("CUMS", "CUM"));
    }

    #[test]
    fn maps_valid_rows_in_order() {
        let range = sheet((0, 0), &[&HEADER, &full_row("A1"), &full_row("B2")]);

        let records = map_range(&range, &test_config()).expect("map").records;

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].cum, "A1");
        assert_eq!(records[0].row_number, 2);
        assert_eq!(records[1].cum, "B2");
        assert_eq!(records[1].row_number, 3);
        assert_eq!(records[1].precio, "100,5");
    }

    #[test]
    fn resolves_columns_by_header_text_not_position() {
        let range = sheet(
            (0, 0),
            &[
                &[
                    "precio",
                    "x",
                    "cum",
                    "COD_GENERICO",
                    "forma_farmaceutica",
                    "concentracion",
                    "unidad_medida",
                    "clasificacion",
                ],
                &["9", "ignored", "C1", "G", "F", "K", "U", "S"],
            ],
        );

        let records = map_range(&range, &test_config()).expect("map").records;

        assert_eq!(records[0].cum, "C1");
        assert_eq!(records[0].precio, "9");
    }

    #[test]
    fn skips_fully_empty_rows() {
        let empty = [""; 7];
        let range = sheet((0, 0), &[&HEADER, &full_row("A1"), &empty, &full_row("B2")]);

        let records = map_range(&range, &test_config()).expect("map").records;

        let rows: Vec<_> = records.iter().map(|r| r.row_number).collect();
        assert_eq!(rows, vec![2, 4]);
    }

    #[test]
    fn row_numbers_are_absolute_when_range_starts_below_a1() {
        // The used range starts at Excel row 10 / column C; header is Excel row 10.
        let range = sheet((9, 2), &[&HEADER, &full_row("A1"), &full_row("B2")]);
        let config = NtImportConfig {
            header_row: 10,
            ..test_config()
        };

        let records = map_range(&range, &config).expect("map").records;

        let rows: Vec<_> = records.iter().map(|r| r.row_number).collect();
        assert_eq!(rows, vec![11, 12]);
    }

    #[test]
    fn rows_above_the_header_row_are_ignored() {
        let range = sheet((0, 0), &[&["title"], &HEADER, &full_row("A1")]);
        let config = NtImportConfig {
            header_row: 2,
            ..test_config()
        };

        let records = map_range(&range, &config).expect("map").records;

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].row_number, 3);
    }

    #[test]
    fn row_with_one_empty_field_is_skipped_with_its_absolute_row() {
        // Header on Excel row 1, 182 valid rows (2..=183), empty CUM on row 184.
        let mut rows = vec![HEADER];
        rows.extend((0..182).map(|_| full_row("X")));
        rows.push(full_row(""));
        let refs: Vec<&[&str]> = rows.iter().map(|r| r.as_slice()).collect();
        let range = sheet((0, 0), &refs);

        let result = map_range(&range, &test_config()).expect("map");

        assert_eq!(result.records.len(), 182);
        assert_eq!(
            result.skipped,
            vec![SkippedRow {
                row_number: 184,
                empty_fields: vec!["CUM"],
            }]
        );
    }

    #[test]
    fn whitespace_only_field_counts_as_empty() {
        let mut row = full_row("A1");
        row[1] = "   ";
        let range = sheet((0, 0), &[&HEADER, &row]);

        let result = map_range(&range, &test_config()).expect("map");

        assert!(result.records.is_empty());
        assert_eq!(
            result.skipped[0].to_string(),
            "Fila 2: campo COD_GENERICO vacío"
        );
    }

    #[test]
    fn row_with_several_empty_fields_lists_all_of_them() {
        let mut row = full_row("");
        row[6] = "";
        let range = sheet((0, 0), &[&HEADER, &full_row("A1"), &row]);

        let result = map_range(&range, &test_config()).expect("map");

        assert_eq!(result.records.len(), 1);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].row_number, 3);
        assert_eq!(result.skipped[0].empty_fields, vec!["CUM", "PRECIO"]);
        assert_eq!(
            result.skipped[0].to_string(),
            "Fila 3: campos CUM, PRECIO vacíos"
        );
    }

    #[test]
    fn fully_empty_rows_are_neither_records_nor_skipped() {
        let empty = [""; 7];
        let range = sheet((0, 0), &[&HEADER, &empty, &full_row("A1")]);

        let result = map_range(&range, &test_config()).expect("map");

        assert_eq!(result.records.len(), 1);
        assert!(result.skipped.is_empty());
    }

    #[test]
    fn records_plus_skipped_account_for_every_non_empty_row() {
        let empty = [""; 7];
        let mut no_price = full_row("C3");
        no_price[6] = "";
        let range = sheet(
            (0, 0),
            &[&HEADER, &full_row("A1"), &empty, &no_price, &full_row("D4")],
        );

        let result = map_range(&range, &test_config()).expect("map");

        assert_eq!(result.records.len() + result.skipped.len(), 3);
        assert_eq!(result.skipped[0].row_number, 4);
    }

    #[test]
    fn missing_single_column_is_named() {
        let range = sheet((0, 0), &[&HEADER[1..], &["a", "b", "c", "d", "e", "f"]]);

        let err = map_range(&range, &test_config()).expect_err("must fail");

        assert_eq!(err.to_string(), "No se encontró la columna CUM");
    }

    #[test]
    fn missing_several_columns_are_all_listed() {
        let range = sheet((0, 0), &[&HEADER[2..], &["a", "b", "c", "d", "e"]]);

        let err = map_range(&range, &test_config()).expect_err("must fail");

        assert_eq!(
            err.to_string(),
            "No se encontraron las columnas CUM, COD_GENERICO"
        );
    }

    #[test]
    fn header_row_outside_the_range_is_an_error() {
        let range = sheet((0, 0), &[&HEADER]);
        let config = NtImportConfig {
            header_row: 5,
            ..test_config()
        };

        let err = map_range(&range, &config).expect_err("must fail");

        assert!(matches!(err, NtImportError::HeaderRowNotFound(5)));
    }

    #[test]
    fn empty_sheet_has_no_header_row() {
        let range: Range<Data> = Range::empty();

        let err = map_range(&range, &test_config()).expect_err("must fail");

        assert!(matches!(err, NtImportError::HeaderRowNotFound(1)));
    }

    #[test]
    fn select_sheet_defaults_to_first_and_reports_unknown_names() {
        let names = vec!["Datos".to_owned(), "Otra".to_owned()];

        assert_eq!(select_sheet(&names, None).expect("first"), "Datos");
        assert_eq!(select_sheet(&names, Some("Otra")).expect("named"), "Otra");
        assert_eq!(
            select_sheet(&names, Some("Nope"))
                .expect_err("missing")
                .to_string(),
            "No se encontró la hoja 'Nope' en la Nota Técnica"
        );
        assert!(matches!(
            select_sheet(&[], None),
            Err(NtImportError::NoSheets)
        ));
    }
}
