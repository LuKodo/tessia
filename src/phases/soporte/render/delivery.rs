//! `Formato_App`: the "CONSTANCIA DE ENTREGA" document, drawn like Go's
//! `deliverypdf` (a port of the JRXML geometry): one fixed 20 pt band per
//! delivery, 18 rows on the first page and 21 on later ones, whole-band page
//! breaks, the column header / signature block / legal text repeated on every
//! page.

use anyhow::{Result, bail};

use super::canvas::{Align, Canvas, Frame};
use super::font;

const HEADING: &str = "CONSTANCIA DE ENTREGA Y RECIBIDO DE MEDICAMENTOS Y/O DISPOSITIVOS MEDICOS";
const RECEIPT: &str = "Certifico la recepción conforme de los medicamentos y/o dispositivos médicos detallados en el presente documento.";

/// The legal paragraph, with the observed Jasper line breaks.
const LEGAL_LINES: [&str; 4] = [
    "Por medio del presente comunicado, nos permitimos informar que DISTRIBUCIONES PHARMASER LTDA, en cumplimiento de lo dispuesto en la Ley 1581 de 2012 y el Decreto",
    "1377 de 2013 sobre la protección de datos personales, informa que los datos suministrados por ustedes serán utilizados únicamente para las finalidades establecidas en nuestra",
    "Política de Tratamiento de Datos Personales. En observancia del régimen integral de protección de datos, solicitamos su autorización para continuar con el tratamiento de su",
    "información personal conforme a los fines allí descritos, los cuales pueden ser consultados en nuestra Política de Tratamiento de Datos Personales.",
];
const WORD_SPACING: [f64; 4] = [0.10689, 0.1056, 0.23727, 0.23727];
const CHAR_SPACING: [f64; 4] = [0.04275, 0.04224, 0.09491, 0.09491];

// Page geometry of Formato_App.jrxml (Letter, 20 pt margins). The title band
// (65 pt) exists on the first page only; the 135 pt column header repeats on
// every page; each 20 pt detail row is a fixed-height band; the column footer
// is anchored at the page bottom (top 591) on every page.
const TITLE_HEIGHT: f64 = 65.0;
const COLUMN_HEADER_TOP: f64 = 20.0;
const COLUMN_HEADER_SIZE: f64 = 135.0;
const DETAIL_HEIGHT: f64 = 20.0;
const COLUMN_FOOTER_TOP: f64 = 591.0;
const DETAIL_TEXT_WIDTH: f64 = 454.0;

/// Bound on the document size; a formula never has this many deliveries.
pub const MAX_ROWS: usize = 1000;

/// One delivery row of the report (the JRXML `soporte_entrega` columns).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeliveryRow {
    /// 1..N within the invoice.
    pub incremento: usize,
    pub invoice: String,
    /// `yyyy-MM-dd` (or empty).
    pub date: String,
    pub authorization: String,
    pub name: String,
    pub doc_type: String,
    pub doc_number: String,
    pub article: String,
    pub quantity: String,
}

/// A validated detail row: display date and quantity.
struct Delivery<'a> {
    row: &'a DeliveryRow,
    date: String,
    quantity: String,
}

/// Renders the document for `rows` with the logo and the patient signature.
pub fn render(rows: &[DeliveryRow], logo: &[u8], signature: &[u8]) -> Result<Vec<u8>> {
    if rows.is_empty() {
        bail!("deliverypdf: at least one delivery row required");
    }
    if rows.len() > MAX_ROWS {
        bail!(
            "deliverypdf: {} delivery rows exceed the limit of {MAX_ROWS}",
            rows.len()
        );
    }
    let deliveries = rows
        .iter()
        .map(|row| {
            Ok(Delivery {
                row,
                date: delivery_date(&row.date)?,
                quantity: delivery_quantity(&row.quantity)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    for (name, data) in [("logo", logo), ("firma", signature)] {
        if data.is_empty() {
            bail!("deliverypdf: missing {name} image");
        }
    }
    let mut canvas = Canvas::new();
    for (index, page) in page_slices(&deliveries).into_iter().enumerate() {
        draw_page(&mut canvas, page, index == 0, logo, signature)?;
    }
    canvas.finish()
}

/// Whole detail bands that fit above the column footer when the column header
/// starts at `header_top`.
fn rows_per_page(header_top: f64) -> usize {
    ((COLUMN_FOOTER_TOP - (header_top + COLUMN_HEADER_SIZE)) / DETAIL_HEIGHT) as usize
}

fn page_slices<'a, 'b>(rows: &'b [Delivery<'a>]) -> Vec<&'b [Delivery<'a>]> {
    let mut pages = Vec::new();
    let mut rest = rows;
    let mut header_top = COLUMN_HEADER_TOP + TITLE_HEIGHT;
    while !rest.is_empty() {
        let n = rows_per_page(header_top).min(rest.len());
        let (page, tail) = rest.split_at(n);
        pages.push(page);
        rest = tail;
        header_top = COLUMN_HEADER_TOP;
    }
    pages
}

struct Cell<'a> {
    text: &'a str,
    frame: Frame,
    size: f64,
    baseline: f64,
    bold: bool,
    align: Align,
}

const fn cell<'a>(text: &'a str, frame: Frame, size: f64, baseline: f64, align: Align) -> Cell<'a> {
    Cell {
        text,
        frame,
        size,
        baseline,
        bold: false,
        align,
    }
}

/// Header fields come from the FIRST row on the page and the column-footer name
/// from the LAST (observed Jasper behavior).
fn draw_page(
    c: &mut Canvas,
    rows: &[Delivery<'_>],
    first: bool,
    logo: &[u8],
    signature: &[u8],
) -> Result<()> {
    let mut dy = 0.0;
    if first {
        c.image(logo, "Logo", Frame::new(20.0, 20.0, 150.0, 50.0), false)
            .map_err(|e| anyhow::anyhow!("deliverypdf: logo: {e:#}"))?;
        c.text(
            HEADING,
            Frame::new(20.0, 69.0, 572.0, 16.0),
            12.0,
            81.54,
            true,
            Align::Center,
        );
    } else {
        dy = -TITLE_HEIGHT;
    }
    draw_column_header(c, &rows[0], dy);
    let top = COLUMN_HEADER_TOP + TITLE_HEIGHT + COLUMN_HEADER_SIZE + dy;
    for (i, delivery) in rows.iter().enumerate() {
        draw_detail(c, delivery, top + i as f64 * DETAIL_HEIGHT);
    }
    draw_column_footer(c, &rows[rows.len() - 1].row.name);
    c.image(
        signature,
        "Signature",
        Frame::new(297.0, 688.0, 284.0, 60.0),
        true,
    )
    .map_err(|e| anyhow::anyhow!("deliverypdf: signature: {e:#}"))?;
    c.end_page();
    Ok(())
}

fn nullable(value: &str) -> String {
    if value.trim().is_empty() {
        "null".into()
    } else {
        value.into()
    }
}

fn draw_column_header(c: &mut Canvas, d: &Delivery<'_>, dy: f64) {
    use Align::{Center, Left};
    let row = d.row;
    let affiliate = format!(
        "{} TIPO DOC: {}  NUM DOC: {}",
        nullable(&row.name),
        nullable(&row.doc_type),
        nullable(&row.doc_number)
    );
    let authorization = nullable(&row.authorization);
    let mut cells = vec![cell(
        &affiliate,
        Frame::new(83.0, 90.0, 503.0, 13.0),
        9.0,
        99.77,
        Left,
    )];
    // Arial-bold labels export as ordinary Helvetica in the observed Jasper PDF.
    let labels = [
        "AFILIADO:",
        "DIRECCION:",
        "ENTIDAD:",
        "CELULAR:",
        "PRESTADOR DEL SERVICIO (PROVEEDOR):",
        "FECHA DE ENTREGA:",
        "AUTORIZACION:",
    ];
    for (i, label) in labels.into_iter().enumerate() {
        let width = match i {
            4 => 196.0,
            5 => 104.0,
            6 => 78.0,
            _ => 59.0,
        };
        let step = i as f64 * 13.0;
        cells.push(cell(
            label,
            Frame::new(24.0, 90.0 + step, width, 13.0),
            9.0,
            99.77 + step,
            Left,
        ));
    }
    cells.extend([
        cell(
            "CARTAGENA",
            Frame::new(83.0, 103.0, 503.0, 13.0),
            9.0,
            112.77,
            Left,
        ),
        cell(
            "MUTUAL SER",
            Frame::new(83.0, 116.0, 503.0, 13.0),
            9.0,
            125.77,
            Left,
        ),
        cell(
            "TELEFONO FIJO:",
            Frame::new(213.0, 129.0, 80.0, 13.0),
            9.0,
            138.77,
            Left,
        ),
        cell(
            "DISTRIBUCIONES PHARMASER LTDA.",
            Frame::new(220.0, 142.0, 366.0, 13.0),
            9.0,
            151.77,
            Left,
        ),
        cell(
            &d.date,
            Frame::new(128.0, 155.0, 458.0, 13.0),
            9.0,
            164.77,
            Left,
        ),
        cell(
            &authorization,
            Frame::new(102.0, 168.0, 484.0, 13.0),
            9.0,
            177.77,
            Left,
        ),
        cell(
            "MEDICAMENTO/ DISPOSTIVO MEDICO",
            Frame::new(26.0, 186.0, 454.0, 33.0),
            9.0,
            205.77,
            Center,
        ),
        cell(
            "UNIDADES",
            Frame::new(482.0, 186.0, 103.0, 33.0),
            9.0,
            200.59,
            Center,
        ),
        cell(
            "ENTREGADAS",
            Frame::new(482.0, 186.0, 103.0, 33.0),
            9.0,
            210.94,
            Center,
        ),
    ]);
    for mut cell in cells {
        cell.frame.y += dy;
        c.text(
            cell.text,
            cell.frame,
            cell.size,
            cell.baseline + dy,
            cell.bold,
            cell.align,
        );
    }
    for mut frame in [
        Frame::new(25.0, 185.0, 456.0, 35.0),
        Frame::new(481.0, 185.0, 105.0, 35.0),
    ] {
        frame.y += dy;
        c.rule_box(frame);
    }
}

/// One 20 pt band whose top is at page-top `y`. The band does not stretch and
/// holds one 8 pt line, so the article is wrapped at the field width and only
/// its first line is shown.
fn draw_detail(c: &mut Canvas, d: &Delivery<'_>, y: f64) {
    let lines = wrap_text(&nullable(&d.row.article), DETAIL_TEXT_WIDTH, 8.0);
    c.text(
        &lines[0],
        Frame::new(26.0, y + 1.0, DETAIL_TEXT_WIDTH, 18.0),
        8.0,
        y + 8.73,
        false,
        Align::Left,
    );
    c.text(
        &d.quantity,
        Frame::new(482.0, y + 1.0, 103.0, 18.0),
        8.0,
        y + 8.73,
        false,
        Align::Center,
    );
    c.rule_box(Frame::new(25.0, y, 456.0, DETAIL_HEIGHT));
    c.rule_box(Frame::new(481.0, y, 105.0, DETAIL_HEIGHT));
}

fn draw_column_footer(c: &mut Canvas, name: &str) {
    use Align::Center;
    for cell in [
        cell(
            "NOMBRE DEL AFILIADO",
            Frame::new(27.0, 650.0, 265.0, 33.0),
            9.0,
            669.77,
            Center,
        ),
        cell(
            "FIRMA DEL AFILIADO Y/O ACUDIENTE",
            Frame::new(294.0, 650.0, 290.0, 33.0),
            9.0,
            669.77,
            Center,
        ),
        cell(
            RECEIPT,
            Frame::new(26.0, 758.0, 560.0, 14.0),
            7.0,
            767.54,
            Center,
        ),
    ] {
        c.text(
            cell.text,
            cell.frame,
            cell.size,
            cell.baseline,
            cell.bold,
            cell.align,
        );
    }
    for frame in [
        Frame::new(26.0, 649.0, 267.0, 35.0),
        Frame::new(293.0, 649.0, 292.0, 35.0),
        Frame::new(26.0, 684.0, 267.0, 69.0),
        Frame::new(293.0, 684.0, 292.0, 69.0),
    ] {
        c.rule_box(frame);
    }
    // Longer names wrap at the fixed field width and remain clipped to its height.
    let lines = wrap_text(&nullable(name), 265.0, 12.0);
    let baseline = 723.04 - (lines.len() as f64 - 1.0) * 6.9;
    for (i, line) in lines.iter().enumerate() {
        c.text(
            line,
            Frame::new(27.0, 685.0, 265.0, 67.0),
            12.0,
            baseline + i as f64 * 13.8,
            false,
            Center,
        );
    }
    for (i, line) in LEGAL_LINES.iter().enumerate() {
        c.spaced_text(
            line,
            Frame::new(25.0, 593.0, 560.0, 44.0),
            7.0,
            599.57 + i as f64 * 8.05,
            WORD_SPACING[i],
            CHAR_SPACING[i],
        );
    }
}

/// Breaks `text` into lines no wider than `width` at the given Helvetica size:
/// at spaces, and inside a single word wider than a whole line (Jasper then
/// breaks between characters).
pub fn wrap_text(text: &str, width: f64, size: f64) -> Vec<String> {
    let measure = |s: &str| font::text_width(s, false, size);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if measure(&candidate) <= width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            line = word.to_owned();
            while measure(&line) > width && line.chars().count() > 1 {
                let chars: Vec<char> = line.chars().collect();
                let mut n = chars.len();
                while n > 1 && measure(&chars[..n].iter().collect::<String>()) > width {
                    n -= 1;
                }
                lines.push(chars[..n].iter().collect());
                line = chars[n..].iter().collect();
            }
        }
        lines.push(line);
    }
    lines
}

/// `dd/MM/yyyy` of a `yyyy-MM-dd` date (first ten characters of a timestamp);
/// an empty date is `null`.
fn delivery_date(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok("null".into());
    }
    let ten: String = value.chars().take(10).collect();
    let parsed = (ten.len() == 10)
        .then(|| chrono::NaiveDate::parse_from_str(&ten, "%Y-%m-%d").ok())
        .flatten();
    match parsed {
        Some(date) => Ok(date.format("%d/%m/%Y").to_string()),
        None => bail!("deliverypdf: invalid delivery date: {ten:?}"),
    }
}

/// Quantity as an integer rounded half-even (`DecimalFormat("###0")`); empty is `null`.
fn delivery_quantity(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok("null".into());
    }
    match value.replace(',', ".").parse::<f64>() {
        Ok(number) if number.is_finite() => Ok(format!("{:.0}", number.round_ties_even())),
        _ => bail!("deliverypdf: invalid quantity {value:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_quantities_are_formatted_like_the_report() {
        assert_eq!(delivery_date("2026-10-05").expect("date"), "05/10/2026");
        assert_eq!(
            delivery_date("2026-10-05 13:00:00").expect("date"),
            "05/10/2026"
        );
        assert_eq!(delivery_date("  ").expect("date"), "null");
        assert!(delivery_date("05/10/2026").is_err());
        assert!(delivery_date("2026-13-01").is_err());
        assert_eq!(delivery_quantity("2").expect("qty"), "2");
        assert_eq!(delivery_quantity("2,5").expect("qty"), "2");
        assert_eq!(delivery_quantity("3.5").expect("qty"), "4");
        assert_eq!(delivery_quantity("").expect("qty"), "null");
        assert!(delivery_quantity("x").is_err());
        assert!(delivery_quantity("NaN").is_err());
    }

    #[test]
    fn pages_hold_18_rows_then_21() {
        let row = DeliveryRow::default();
        let deliveries: Vec<Delivery<'_>> = (0..40)
            .map(|_| Delivery {
                row: &row,
                date: String::new(),
                quantity: String::new(),
            })
            .collect();
        let sizes: Vec<usize> = page_slices(&deliveries).iter().map(|p| p.len()).collect();
        assert_eq!(sizes, [18, 21, 1]);
        assert_eq!(page_slices(&deliveries[..18]).len(), 1);
        assert_eq!(page_slices(&deliveries[..19]).len(), 2);
    }

    #[test]
    fn wrapping_breaks_at_spaces_and_inside_long_words() {
        assert_eq!(wrap_text("uno dos", 1000.0, 8.0), ["uno dos"]);
        let lines = wrap_text("aaaa bbbb cccc dddd", 25.0, 8.0);
        assert!(lines.len() > 1);
        assert_eq!(lines.join(" "), "aaaa bbbb cccc dddd");
        let long = "W".repeat(40);
        let lines = wrap_text(&long, 50.0, 8.0);
        assert!(lines.len() > 1);
        assert_eq!(lines.concat(), long);
        assert!(
            lines
                .iter()
                .all(|l| font::text_width(l, false, 8.0) <= 50.0)
        );
        assert_eq!(wrap_text("", 50.0, 8.0), [""]);
    }
}
