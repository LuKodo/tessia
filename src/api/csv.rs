//! CSV output helpers (Go: `csvCell` with formula neutralization).
//!
//! Both dashboard exports use a UTF-8 BOM, `;` as separator and CRLF line
//! endings, so a spreadsheet opens them directly.

use super::request::Response;

/// The UTF-8 byte order mark every export starts with.
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// Characters that make a spreadsheet read a cell as a formula.
const FORMULA_STARTS: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// A cell ready to be written: a leading `'` neutralizes a formula (the cell
/// stays text) and a cell holding `;`, `"`, CR or LF is quoted with `"`
/// doubled.
///
/// ```
/// use tessia::api::csv::csv_cell;
/// assert_eq!(csv_cell("=1+1"), "'=1+1");
/// assert_eq!(csv_cell("a;b"), "\"a;b\"");
/// assert_eq!(csv_cell("plain"), "plain");
/// ```
pub fn csv_cell(text: &str) -> String {
    let mut cell = if text.starts_with(FORMULA_STARTS) {
        format!("'{text}")
    } else {
        text.to_owned()
    };
    if cell.contains([';', '"', '\r', '\n']) {
        cell = format!("\"{}\"", cell.replace('"', "\"\""));
    }
    cell
}

/// Builds a CSV document row by row.
#[derive(Debug, Clone)]
pub struct CsvWriter {
    buf: Vec<u8>,
}

impl CsvWriter {
    /// A document that already holds the BOM.
    pub fn new() -> Self {
        Self { buf: BOM.to_vec() }
    }

    /// Appends one row; every cell goes through [`csv_cell`].
    pub fn row<S: AsRef<str>>(&mut self, cells: &[S]) {
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                self.buf.push(b';');
            }
            self.buf
                .extend_from_slice(csv_cell(cell.as_ref()).as_bytes());
        }
        self.buf.extend_from_slice(b"\r\n");
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

impl Default for CsvWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Keeps a name usable inside a `Content-Disposition` filename: everything but
/// `[A-Za-z0-9._-]` becomes `_` (Go `safeFileName`).
pub fn safe_file_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A 200 CSV download named `filename` (already safe for a header value).
pub fn download(filename: &str, body: Vec<u8>) -> Response {
    let mut response = Response::new(200)
        .with_header("Content-Type", "text/csv; charset=utf-8")
        .with_header(
            "Content-Disposition",
            &format!("attachment; filename=\"{filename}\""),
        );
    response.body = body;
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_starts_are_neutralized() {
        for start in ["=SUM(A1)", "+1", "-1", "@cmd", "\tx", "\rx"] {
            assert!(
                csv_cell(start).trim_matches('"').starts_with('\''),
                "{start:?}"
            );
        }
        assert_eq!(csv_cell(""), "");
        assert_eq!(csv_cell("a=b"), "a=b");
    }

    #[test]
    fn cells_with_separators_quotes_or_breaks_are_quoted() {
        assert_eq!(csv_cell("a;b"), "\"a;b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("a\nb"), "\"a\nb\"");
        // The neutralizing quote is added first, then the cell is quoted.
        assert_eq!(csv_cell("\rx"), "\"'\rx\"");
        assert_eq!(csv_cell("a,b"), "a,b");
    }

    #[test]
    fn the_writer_emits_bom_semicolons_and_crlf() {
        let mut csv = CsvWriter::new();
        csv.row(&["Factura", "Motivo"]);
        csv.row(&["FEV1", "=x;y"]);
        let out = csv.finish();
        assert_eq!(&out[..3], b"\xEF\xBB\xBF");
        assert_eq!(
            String::from_utf8_lossy(&out[3..]),
            "Factura;Motivo\r\nFEV1;\"'=x;y\"\r\n"
        );
    }

    #[test]
    fn file_names_keep_only_safe_characters() {
        assert_eq!(safe_file_name("Convenio_01-02.v2"), "Convenio_01-02.v2");
        assert_eq!(safe_file_name("a b/ñ\"c"), "a_b___c");
    }

    #[test]
    fn a_download_names_the_file_and_declares_the_charset() {
        let response = download("x.csv", b"body".to_vec());
        assert_eq!(response.status, 200);
        assert_eq!(
            response.header("content-type"),
            Some("text/csv; charset=utf-8")
        );
        assert_eq!(
            response.header("content-disposition"),
            Some("attachment; filename=\"x.csv\"")
        );
        assert_eq!(response.body, b"body");
    }
}
