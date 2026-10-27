//! Reads the first sheet of an `.xlsx` workbook and renders it as the `;`
//! separated CSV that [`super::csv::read_csv`] expects.
//!
//! Only what a convenio export needs is supported: shared and inline strings,
//! plain numbers, booleans and dates (a numeric cell whose number format is a
//! date is written as `yyyy-mm-dd`). Formulas are read through their cached
//! value. The legacy `.xls` format is not supported.
//!
//! The XML is read with `quick-xml` and the container with `zip`, both already
//! dependencies, so no spreadsheet crate is needed.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use anyhow::{Context, Result, bail};
use chrono::{Duration, NaiveDate};
use quick_xml::Reader;
use quick_xml::events::{BytesRef, BytesStart, Event};
use zip::ZipArchive;
use zip::result::ZipError;

/// Cap on the bytes read from one entry of the workbook (zip-bomb guard).
const MAX_ENTRY_BYTES: u64 = 256 << 20;
/// Upper bound of columns in a sheet (Excel's own limit, `XFD`).
const MAX_COLUMNS: usize = 16_384;
const SEPARATOR: char = ';';

/// Renders the first sheet of the workbook (in tab order) as CSV bytes.
///
/// The first non-empty row is the header; fully empty rows are dropped and
/// every row is padded or cut to the header's width. Values lose the CSV
/// separators and line breaks, which the convenio reader cannot represent.
pub fn first_sheet_to_csv(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|err| anyhow::anyhow!("convenio: xlsx: not a valid workbook: {err}"))?;

    let workbook = entry(&mut archive, "xl/workbook.xml")?
        .context("convenio: xlsx: the file has no xl/workbook.xml")?;
    let (first_sheet, date1904) = parse_workbook(&workbook)?;
    let sheet_path = match first_sheet {
        Some(rel_id) => {
            let rels = entry(&mut archive, "xl/_rels/workbook.xml.rels")?.unwrap_or_default();
            sheet_target(&rels, &rel_id)?
        }
        None => bail!("convenio: xlsx: the workbook has no sheets"),
    };
    let sheet = entry(&mut archive, &sheet_path)?
        .with_context(|| format!("convenio: xlsx: the first sheet {sheet_path} is missing"))?;

    let shared = match entry(&mut archive, "xl/sharedStrings.xml")? {
        Some(xml) => parse_shared_strings(&xml)?,
        None => Vec::new(),
    };
    let date_styles = match entry(&mut archive, "xl/styles.xml")? {
        Some(xml) => parse_date_styles(&xml)?,
        None => Vec::new(),
    };

    let rows = parse_sheet(&sheet, &shared, &date_styles, date1904)?;
    render_csv(rows)
}

fn entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>> {
    let file = match archive.by_name(name) {
        Ok(file) => file,
        Err(ZipError::FileNotFound) => return Ok(None),
        Err(err) => bail!("convenio: xlsx: cannot open {name}: {err}"),
    };
    let mut buf = Vec::new();
    file.take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut buf)
        .with_context(|| format!("convenio: xlsx: cannot read {name}"))?;
    if buf.len() as u64 > MAX_ENTRY_BYTES {
        bail!("convenio: xlsx: {name} is too large");
    }
    Ok(Some(buf))
}

/// What the XML walker hands to a handler. Names are namespace-less.
enum Item<'a> {
    Open(&'a BytesStart<'a>),
    Close(&'a [u8]),
    Text(&'a str),
}

fn walk(xml: &[u8], mut handle: impl FnMut(Item<'_>) -> Result<()>) -> Result<()> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .context("convenio: xlsx: malformed XML")?
        {
            Event::Start(e) => handle(Item::Open(&e))?,
            Event::Empty(e) => {
                handle(Item::Open(&e))?;
                handle(Item::Close(e.local_name().as_ref()))?;
            }
            Event::End(e) => handle(Item::Close(e.local_name().as_ref()))?,
            Event::Text(t) => handle(Item::Text(&t.decode().context("convenio: xlsx: text")?))?,
            Event::CData(t) => handle(Item::Text(&t.decode().context("convenio: xlsx: text")?))?,
            Event::GeneralRef(r) => handle(Item::Text(&resolve_ref(&r)?))?,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn resolve_ref(r: &BytesRef<'_>) -> Result<String> {
    if let Some(c) = r
        .resolve_char_ref()
        .context("convenio: xlsx: bad character reference")?
    {
        return Ok(c.to_string());
    }
    let name = r.decode().context("convenio: xlsx: bad entity")?;
    Ok(match name.as_ref() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        other => bail!("convenio: xlsx: unknown entity &{other};"),
    }
    .to_owned())
}

fn attr(e: &BytesStart<'_>, key: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == key.as_bytes())
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

/// Relationship id of the first `<sheet>` and whether dates use the 1904 base.
fn parse_workbook(xml: &[u8]) -> Result<(Option<String>, bool)> {
    let mut first = None;
    let mut date1904 = false;
    walk(xml, |item| {
        if let Item::Open(e) = item {
            match e.local_name().as_ref() {
                b"sheet" if first.is_none() => first = attr(e, "id"),
                b"workbookPr" => {
                    date1904 = matches!(attr(e, "date1904").as_deref(), Some("1" | "true"));
                }
                _ => {}
            }
        }
        Ok(())
    })?;
    Ok((first, date1904))
}

/// Path inside the archive of the part a workbook relationship points to.
fn sheet_target(rels: &[u8], rel_id: &str) -> Result<String> {
    let mut target = None;
    walk(rels, |item| {
        if let Item::Open(e) = item
            && e.local_name().as_ref() == b"Relationship"
            && attr(e, "Id").as_deref() == Some(rel_id)
        {
            target = attr(e, "Target");
        }
        Ok(())
    })?;
    let target = target.with_context(|| format!("convenio: xlsx: no sheet for {rel_id}"))?;
    Ok(match target.strip_prefix('/') {
        Some(absolute) => absolute.to_owned(),
        None => format!("xl/{target}"),
    })
}

fn parse_shared_strings(xml: &[u8]) -> Result<Vec<String>> {
    let mut strings = Vec::new();
    let mut current = String::new();
    let (mut in_si, mut in_t, mut phonetic) = (false, false, 0_u32);
    walk(xml, |item| {
        match item {
            Item::Open(e) => match e.local_name().as_ref() {
                b"si" => {
                    in_si = true;
                    current.clear();
                }
                b"rPh" => phonetic += 1,
                b"t" if in_si && phonetic == 0 => in_t = true,
                _ => {}
            },
            Item::Close(name) => match name {
                b"si" => {
                    in_si = false;
                    strings.push(std::mem::take(&mut current));
                }
                b"rPh" => phonetic = phonetic.saturating_sub(1),
                b"t" => in_t = false,
                _ => {}
            },
            Item::Text(text) => {
                if in_t {
                    current.push_str(text);
                }
            }
        }
        Ok(())
    })?;
    Ok(strings)
}

/// For each cell format (`cellXfs`, by index): is it a date/time format?
fn parse_date_styles(xml: &[u8]) -> Result<Vec<bool>> {
    let mut custom: HashMap<u32, String> = HashMap::new();
    let mut xf_ids: Vec<u32> = Vec::new();
    let mut in_cell_xfs = false;
    walk(xml, |item| {
        match item {
            Item::Open(e) => match e.local_name().as_ref() {
                b"numFmt" => {
                    if let (Some(id), Some(code)) = (attr(e, "numFmtId"), attr(e, "formatCode"))
                        && let Ok(id) = id.parse()
                    {
                        custom.insert(id, code);
                    }
                }
                b"cellXfs" => in_cell_xfs = true,
                b"xf" if in_cell_xfs => {
                    xf_ids.push(
                        attr(e, "numFmtId")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0),
                    );
                }
                _ => {}
            },
            Item::Close(b"cellXfs") => in_cell_xfs = false,
            _ => {}
        }
        Ok(())
    })?;
    Ok(xf_ids
        .into_iter()
        .map(|id| match custom.get(&id) {
            Some(code) => is_date_code(code),
            None => matches!(id, 14..=22 | 27..=36 | 45..=47 | 50..=58),
        })
        .collect())
}

/// Whether a custom number format code shows a date or a time.
fn is_date_code(code: &str) -> bool {
    let mut visible = String::new();
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                for inner in chars.by_ref() {
                    if inner == '"' {
                        break;
                    }
                }
            }
            '[' => {
                for inner in chars.by_ref() {
                    if inner == ']' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            other => visible.push(other.to_ascii_lowercase()),
        }
    }
    visible
        .chars()
        .any(|c| matches!(c, 'y' | 'd' | 'h' | 'm' | 's'))
}

/// A cell being read: its column, type attribute, style and raw contents.
#[derive(Default)]
struct Cell {
    column: usize,
    kind: String,
    date: bool,
    value: String,
    inline: String,
}

fn parse_sheet(
    xml: &[u8],
    shared: &[String],
    date_styles: &[bool],
    date1904: bool,
) -> Result<Vec<Vec<String>>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell: Option<Cell> = None;
    let mut next_column = 0_usize;
    let (mut in_v, mut in_is, mut in_t, mut phonetic) = (false, false, false, 0_u32);
    walk(xml, |item| {
        match item {
            Item::Open(e) => match e.local_name().as_ref() {
                b"row" => {
                    row.clear();
                    next_column = 0;
                }
                b"c" => {
                    let column = attr(e, "r")
                        .and_then(|reference| column_of(&reference))
                        .unwrap_or(next_column);
                    if column >= MAX_COLUMNS {
                        bail!("convenio: xlsx: the sheet has too many columns");
                    }
                    let style: usize = attr(e, "s").and_then(|s| s.parse().ok()).unwrap_or(0);
                    cell = Some(Cell {
                        column,
                        kind: attr(e, "t").unwrap_or_default(),
                        date: date_styles.get(style).copied().unwrap_or(false),
                        ..Cell::default()
                    });
                }
                b"v" => in_v = true,
                b"is" => in_is = true,
                b"rPh" => phonetic += 1,
                b"t" if in_is && phonetic == 0 => in_t = true,
                _ => {}
            },
            Item::Close(name) => match name {
                b"v" => in_v = false,
                b"is" => in_is = false,
                b"rPh" => phonetic = phonetic.saturating_sub(1),
                b"t" => in_t = false,
                b"c" => {
                    if let Some(done) = cell.take() {
                        next_column = done.column + 1;
                        let column = done.column;
                        let text = cell_text(done, shared, date1904);
                        if row.len() <= column {
                            row.resize(column + 1, String::new());
                        }
                        row[column] = text;
                    }
                }
                b"row" => rows.push(std::mem::take(&mut row)),
                _ => {}
            },
            Item::Text(text) => {
                if let Some(current) = cell.as_mut() {
                    if in_v {
                        current.value.push_str(text);
                    } else if in_t {
                        current.inline.push_str(text);
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(rows)
}

/// Zero-based column of an `A1`-style reference.
fn column_of(reference: &str) -> Option<usize> {
    let mut column = 0_usize;
    let mut letters = 0;
    for c in reference.chars().take_while(char::is_ascii_alphabetic) {
        column = column
            .checked_mul(26)?
            .checked_add(usize::from(c.to_ascii_uppercase() as u8 - b'A') + 1)?;
        letters += 1;
    }
    (letters > 0).then(|| column - 1)
}

fn cell_text(cell: Cell, shared: &[String], date1904: bool) -> String {
    match cell.kind.as_str() {
        "s" => cell
            .value
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|index| shared.get(index).cloned())
            .unwrap_or_default(),
        "inlineStr" => cell.inline,
        "str" => cell.value,
        "b" => if cell.value.trim() == "1" {
            "TRUE"
        } else {
            "FALSE"
        }
        .to_owned(),
        "e" => String::new(),
        _ => {
            let raw = cell.value.trim();
            if cell.date
                && let Some(date) = raw
                    .parse::<f64>()
                    .ok()
                    .and_then(|n| excel_date(n, date1904))
            {
                return date;
            }
            raw.to_owned()
        }
    }
}

/// `yyyy-mm-dd` (plus ` hh:mm:ss` when the serial has a time of day).
fn excel_date(serial: f64, date1904: bool) -> Option<String> {
    if !serial.is_finite() || !(0.0..1.0e7).contains(&serial) {
        return None;
    }
    let mut days = serial.floor() as i64;
    let mut seconds = ((serial - serial.floor()) * 86_400.0).round() as i64;
    if seconds >= 86_400 {
        days += 1;
        seconds -= 86_400;
    }
    let base = if date1904 {
        NaiveDate::from_ymd_opt(1904, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(1899, 12, 30)?
    };
    let date = base.checked_add_signed(Duration::days(days))?;
    Some(if seconds == 0 {
        date.format("%Y-%m-%d").to_string()
    } else {
        let time = date.and_hms_opt(0, 0, 0)? + Duration::seconds(seconds);
        time.format("%Y-%m-%d %H:%M:%S").to_string()
    })
}

fn render_csv(rows: Vec<Vec<String>>) -> Result<Vec<u8>> {
    let mut rows = rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|value| clean(&value))
                .collect::<Vec<String>>()
        })
        .filter(|row| row.iter().any(|value| !value.is_empty()));
    let header = rows
        .next()
        .context("convenio: xlsx: the first sheet is empty")?;
    let width = header
        .iter()
        .rposition(|value| !value.is_empty())
        .map_or(0, |last| last + 1);
    let mut out = String::new();
    let mut push = |mut row: Vec<String>| {
        row.resize(width, String::new());
        out.push_str(&row.join(&SEPARATOR.to_string()));
        out.push('\n');
    };
    push(header);
    rows.for_each(push);
    Ok(out.into_bytes())
}

/// A value as the convenio reader can carry it: trimmed, with the separators
/// (`;`, `|`) and line breaks turned into spaces.
fn clean(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if matches!(c, ';' | '|' | '\r' | '\n') {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::*;
    use crate::phases::convenio_preparar::csv::read_csv;

    fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, body) in entries {
            writer.start_file(*name, options).expect("start");
            writer.write_all(body.as_bytes()).expect("write");
        }
        writer.finish().expect("finish").into_inner()
    }

    const WORKBOOK: &str = r#"<workbook xmlns="x" xmlns:r="r"><workbookPr/><sheets>
        <sheet name="Primera" sheetId="7" r:id="rId2"/>
        <sheet name="Segunda" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    const RELS: &str = r#"<Relationships>
        <Relationship Id="rId1" Target="worksheets/sheet1.xml"/>
        <Relationship Id="rId2" Target="worksheets/sheet2.xml"/></Relationships>"#;
    const STYLES: &str = r#"<styleSheet>
        <numFmts count="1"><numFmt numFmtId="170" formatCode="yyyy\-mm\-dd"/></numFmts>
        <cellStyleXfs count="1"><xf numFmtId="14"/></cellStyleXfs>
        <cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="170"/></cellXfs>
        </styleSheet>"#;
    const STRINGS: &str = r#"<sst><si><t>Factura</t></si><si><t>Fecha</t></si>
        <si><r><t>Art</t></r><r><t> a;b</t></r><rPh><t>ignored</t></rPh></si>
        <si><t>Tom &amp; Jerry</t></si></sst>"#;

    fn workbook(first_sheet: &str) -> Vec<u8> {
        zip(&[
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", RELS),
            ("xl/styles.xml", STYLES),
            ("xl/sharedStrings.xml", STRINGS),
            ("xl/worksheets/sheet2.xml", first_sheet),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Otra hoja</t></is></c></row></sheetData></worksheet>"#,
            ),
        ])
    }

    fn csv_of(first_sheet: &str) -> String {
        String::from_utf8(first_sheet_to_csv(&workbook(first_sheet)).expect("csv")).expect("utf8")
    }

    #[test]
    fn reads_the_first_sheet_in_tab_order_not_the_first_file() {
        let csv = csv_of(
            r#"<worksheet><sheetData>
            <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
            <row r="2"><c r="A2" t="inlineStr"><is><t>FV1</t></is></c><c r="B2" s="1"><v>46090</v></c></row>
            </sheetData></worksheet>"#,
        );
        assert_eq!(csv, "Factura;Fecha\nFV1;2026-03-09\n");
    }

    #[test]
    fn dates_come_from_the_number_format_and_other_numbers_stay_raw() {
        let csv = csv_of(
            r#"<worksheet><sheetData>
            <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="s"><v>0</v></c></row>
            <row r="2"><c r="A2"><v>1048613825</v></c><c r="B2" s="2"><v>46090.5</v></c><c r="C2" s="0"><v>46090</v></c></row>
            </sheetData></worksheet>"#,
        );
        assert_eq!(
            csv,
            "Factura;Fecha;Factura\n1048613825;2026-03-09 12:00:00;46090\n"
        );
    }

    #[test]
    fn separators_line_breaks_entities_and_phonetic_text_are_handled() {
        let csv = csv_of(
            r#"<worksheet><sheetData>
            <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
            <row r="2"><c r="A2" t="s"><v>2</v></c><c r="B2" t="s"><v>3</v></c></row>
            <row r="3"><c r="A3" t="str"><v>x|y</v></c><c r="B3" t="b"><v>1</v></c></row>
            </sheetData></worksheet>"#,
        );
        assert_eq!(csv, "Factura;Fecha\nArt a b;Tom & Jerry\nx y;TRUE\n");
    }

    #[test]
    fn gaps_empty_rows_and_columns_past_the_header_are_normalised() {
        let csv = csv_of(
            r#"<worksheet><sheetData>
            <row r="1"><c r="B1" t="inlineStr"><is><t>b</t></is></c><c r="C1" t="inlineStr"><is><t>c</t></is></c></row>
            <row r="2"/>
            <row r="3"><c r="C3" t="inlineStr"><is><t>z</t></is></c><c r="F3" t="inlineStr"><is><t>extra</t></is></c></row>
            <row r="4"><c r="A4" t="e"><v>#N/A</v></c></row>
            </sheetData></worksheet>"#,
        );
        assert_eq!(csv, ";b;c\n;;z\n");
    }

    #[test]
    fn the_output_is_a_csv_the_convenio_reader_accepts() {
        let csv = csv_of(
            r#"<worksheet><sheetData>
            <row r="1"><c r="A1" t="inlineStr"><is><t>Factura</t></is></c>
              <c r="B1" t="inlineStr"><is><t>Tipo de identificación</t></is></c>
              <c r="C1" t="inlineStr"><is><t>No. Documento</t></is></c>
              <c r="D1" t="inlineStr"><is><t>Fecha</t></is></c>
              <c r="E1" t="inlineStr"><is><t>PLU</t></is></c>
              <c r="F1" t="inlineStr"><is><t>Artículo</t></is></c>
              <c r="G1" t="inlineStr"><is><t>Cantidad</t></is></c>
              <c r="H1" t="inlineStr"><is><t>Vlr. unitario</t></is></c>
              <c r="I1" t="inlineStr"><is><t>Vlr. total</t></is></c>
              <c r="J1" t="inlineStr"><is><t>Cuota</t></is></c>
              <c r="K1" t="inlineStr"><is><t>CUM</t></is></c></row>
            <row r="2"><c r="A2" t="inlineStr"><is><t>FVS1</t></is></c>
              <c r="B2" t="inlineStr"><is><t>TI</t></is></c><c r="C2"><v>1048613825</v></c>
              <c r="D2" s="1"><v>46090</v></c><c r="E2" t="inlineStr"><is><t>823_11_102</t></is></c>
              <c r="F2" t="inlineStr"><is><t>HIDROXIUREA</t></is></c><c r="G2"><v>100</v></c>
              <c r="H2"><v>1924</v></c><c r="I2"><v>192400</v></c><c r="J2"><v>0</v></c>
              <c r="K2" t="inlineStr"><is><t>1H1046221005100</t></is></c></row>
            </sheetData></worksheet>"#,
        );
        let read = read_csv(Cursor::new(csv.into_bytes())).expect("read");
        assert_eq!((read.lines.len(), read.skipped), (1, 0));
        assert_eq!(read.lines[0].date, "2026-03-09");
        assert_eq!(read.lines[0].doc_number, "1048613825");
    }

    #[test]
    fn broken_workbooks_are_errors() {
        assert!(
            first_sheet_to_csv(b"factura;plu\n1;2\n").is_err(),
            "not a zip"
        );
        assert!(
            first_sheet_to_csv(&zip(&[("a.txt", "x")])).is_err(),
            "no workbook"
        );
        let empty = zip(&[
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", RELS),
            (
                "xl/worksheets/sheet2.xml",
                "<worksheet><sheetData/></worksheet>",
            ),
        ]);
        let err = first_sheet_to_csv(&empty).expect_err("empty sheet");
        assert!(err.to_string().contains("empty"), "{err}");
    }

    #[test]
    fn column_references_and_date_codes() {
        assert_eq!(column_of("A1"), Some(0));
        assert_eq!(column_of("Z9"), Some(25));
        assert_eq!(column_of("AA10"), Some(26));
        assert_eq!(column_of("BN2"), Some(65));
        assert_eq!(column_of("12"), None);
        assert!(is_date_code("dd/mm/yyyy"));
        assert!(is_date_code("[$-409]d-mmm-yy;@"));
        assert!(!is_date_code("General"));
        assert!(!is_date_code(r##""$"#,##0.00"##));
        assert!(!is_date_code("0.00E+00"));
    }
}
