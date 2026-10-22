//! Parsing of the Medicar portal pages (port of the HTML half of Go
//! `internal/adapters/medicar/formula.go`, itself modelled on the PHP
//! `MedicarFormulaClient`).
//!
//! The pages are loose HTML: unclosed `<td>`, entities, Windows-1252 bodies.
//! Everything here is a pure function over text.

use std::sync::LazyLock;

use regex::Regex;
use rust_decimal::Decimal;

use crate::util::text::normalize_key;

/// What Medicar reports for a formula. Only `fecha_formula` is used by the
/// reconciliation; the rest is evidence.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Formula {
    /// `yyyy-MM-dd[ HH:mm:ss]`.
    pub fecha_formula: String,
    pub molecula: String,
    pub articulo_medicar: String,
    pub estado_despacho: String,
    pub fecha_despacho: String,
    pub cantidad: Decimal,
    pub documento: String,
    pub factura: String,
    pub ssc: String,
}

/// Why a page could not be used. Every case is a permanent error (the
/// invoice goes to manual review); the message is the Go text without the
/// `medicar: ` prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageError(pub String);

fn regex(pattern: &str) -> Regex {
    match Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => panic!("conciliar: invalid built-in pattern {pattern:?}: {e}"),
    }
}

/// ASCII whitespace as Go's `\s` (the regex crate's `\s` is Unicode).
const WS: &str = r"[\t\n\x0C\r ]";

static CSRF: LazyLock<Regex> =
    LazyLock::new(|| regex(r#"(?i)name=["']csrf_token["'][^>]*value=["']([^"']+)["']"#));
static CSRF_ALT: LazyLock<Regex> =
    LazyLock::new(|| regex(r#"(?i)value=["']([^"']+)["'][^>]*name=["']csrf_token["']"#));
static ALNUM_OUT: LazyLock<Regex> = LazyLock::new(|| regex(r"[^A-Z0-9]"));
static FECHA: LazyLock<Regex> = LazyLock::new(|| {
    regex(&format!(
        r"(?i)Fecha:{WS}*([0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}(?:{WS}+[0-9]{{2}}:[0-9]{{2}}:[0-9]{{2}})?)"
    ))
});
static DISPATCH: LazyLock<Regex> = LazyLock::new(|| {
    regex(&format!(
        r"[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}(?:{WS}+[0-9]{{2}}:[0-9]{{2}}:[0-9]{{2}})?"
    ))
});
static FORMULA_NO_HEADER: LazyLock<Regex> =
    LazyLock::new(|| regex(&format!(r"(?i)Formula{WS}+SSC{WS}+No:")));
static SSC_BARE_HEADER: LazyLock<Regex> = LazyLock::new(|| regex(r"(?i)SSC"));
static OPEN_FORMULA_HEAD: LazyLock<Regex> =
    LazyLock::new(|| regex(&format!(r"(?i)AjaxModalVerFormula{WS}*\(")));
static TR_OPEN: LazyLock<Regex> = LazyLock::new(|| regex(r"(?i)<tr(?-u:\b)"));
static TR_CLOSE: LazyLock<Regex> = LazyLock::new(|| regex(&format!(r"(?i)</{WS}*tr{WS}*>")));
static TD_OPEN: LazyLock<Regex> = LazyLock::new(|| regex(r"(?i)<t[dh](?-u:\b)[^>]*>"));
static TD_CLOSE: LazyLock<Regex> = LazyLock::new(|| regex(&format!(r"(?i)</{WS}*t[dh]{WS}*>")));
static BR: LazyLock<Regex> = LazyLock::new(|| regex(&format!(r"(?i)<br{WS}*/?{WS}*>")));
static TAG: LazyLock<Regex> = LazyLock::new(|| regex(r"<[^>]*>"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| regex(r"[\t\n\x0C\r \x{00A0}]+"));
static NON_NAME: LazyLock<Regex> = LazyLock::new(|| regex(r"[^A-Z0-9]+"));

/// The (entity-decoded) `csrf_token` of the login page, with the attributes in
/// either order.
pub fn find_csrf(page: &str) -> Option<String> {
    let caps = CSRF.captures(page).or_else(|| CSRF_ALT.captures(page))?;
    Some(unescape_html(caps.get(1)?.as_str()))
}

// ---------------------------------------------------------------- encoding

/// Windows-1252 code points of the bytes 0x80..=0x9F.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}', '\u{178}',
];

/// UTF-8 text; a body that is not valid UTF-8 is read as Windows-1252
/// (`MedicarFormulaClient.utf8`).
pub fn decode_portal(raw: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(raw) {
        return s.to_owned();
    }
    raw.iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252_HIGH[usize::from(b - 0x80)],
            other => char::from(other),
        })
        .collect()
}

/// Names of the HTML entities for U+00A0..=U+00FF, in code point order.
const LATIN1_ENTITIES: [&str; 96] = [
    "nbsp", "iexcl", "cent", "pound", "curren", "yen", "brvbar", "sect", "uml", "copy", "ordf",
    "laquo", "not", "shy", "reg", "macr", "deg", "plusmn", "sup2", "sup3", "acute", "micro",
    "para", "middot", "cedil", "sup1", "ordm", "raquo", "frac14", "frac12", "frac34", "iquest",
    "Agrave", "Aacute", "Acirc", "Atilde", "Auml", "Aring", "AElig", "Ccedil", "Egrave", "Eacute",
    "Ecirc", "Euml", "Igrave", "Iacute", "Icirc", "Iuml", "ETH", "Ntilde", "Ograve", "Oacute",
    "Ocirc", "Otilde", "Ouml", "times", "Oslash", "Ugrave", "Uacute", "Ucirc", "Uuml", "Yacute",
    "THORN", "szlig", "agrave", "aacute", "acirc", "atilde", "auml", "aring", "aelig", "ccedil",
    "egrave", "eacute", "ecirc", "euml", "igrave", "iacute", "icirc", "iuml", "eth", "ntilde",
    "ograve", "oacute", "ocirc", "otilde", "ouml", "divide", "oslash", "ugrave", "uacute", "ucirc",
    "uuml", "yacute", "thorn", "yuml",
];

fn named_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "ndash" => Some('\u{2013}'),
        "mdash" => Some('\u{2014}'),
        "hellip" => Some('\u{2026}'),
        "euro" => Some('\u{20AC}'),
        _ => LATIN1_ENTITIES
            .iter()
            .position(|n| *n == name)
            .and_then(|i| char::from_u32(0xA0 + u32::try_from(i).ok()?)),
    }
}

fn numeric_entity(body: &str) -> Option<char> {
    let (digits, radix) = match body.strip_prefix(['x', 'X']) {
        Some(hex) => (hex, 16),
        None => (body, 10),
    };
    char::from_u32(u32::from_str_radix(digits, radix).ok()?)
}

/// Decodes numeric and the common named entities (`&amp;`, `&nbsp;`, the
/// Latin-1 letters); an unknown `&...;` is left as is.
pub fn unescape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest[1..]
            .find(';')
            .filter(|end| *end <= 10)
            .and_then(|end| {
                let body = &rest[1..=end];
                let c = match body.strip_prefix('#') {
                    Some(num) => numeric_entity(num),
                    None => named_entity(body),
                }?;
                Some((c, end + 2))
            });
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// -------------------------------------------------------------- text tools

/// Strips tags, decodes entities and collapses spaces (NBSP included).
pub fn page_text(s: &str) -> String {
    let no_br = BR.replace_all(s, " ");
    let no_tags = TAG.replace_all(&no_br, "");
    let decoded = unescape_html(&no_tags);
    SPACES.replace_all(&decoded, " ").trim().to_owned()
}

/// Folds accents, upper-cases and keeps `[A-Z0-9]` words.
pub fn normalize_name(s: &str) -> String {
    let folded = normalize_key(&page_text(s));
    NON_NAME.replace_all(&folded, " ").trim().to_owned()
}

/// Splits `s` at every `open` match; each piece ends at the first `close`
/// match, the next open or the end (tolerates unclosed tags like the PHP
/// regex). Returns the whole pieces and their inner content.
fn segments(s: &str, open: &Regex, close: &Regex) -> (Vec<String>, Vec<String>) {
    let locs: Vec<_> = open.find_iter(s).collect();
    let mut whole = Vec::with_capacity(locs.len());
    let mut inner = Vec::with_capacity(locs.len());
    for (i, l) in locs.iter().enumerate() {
        let end = locs.get(i + 1).map_or(s.len(), |n| n.start());
        let mut content = &s[l.end()..end];
        let mut stop = end;
        if let Some(c) = close.find(content) {
            stop = l.end() + c.end();
            content = &content[..c.start()];
        }
        whole.push(s[l.start()..stop].to_owned());
        inner.push(content.to_owned());
    }
    (whole, inner)
}

fn rows(page: &str) -> Vec<String> {
    segments(page, &TR_OPEN, &TR_CLOSE).0
}

fn cells(row: &str) -> Vec<String> {
    segments(row, &TD_OPEN, &TD_CLOSE)
        .1
        .iter()
        .map(|c| page_text(c))
        .collect()
}

// ------------------------------------------------------- SSC hand matchers
//
// They reproduce, without compiling a pattern per call, what these patterns
// matched in Go (q = the escaped SSC):
//
//   (?i)Formula\s+SSC\s+No:\s*q\b
//   (?i)SSC\s*q\b
//   (?i)AjaxModalVerFormula\s*\(\s*["']?q["']?\s*\)
//
// The SSC is literal but case-insensitive (Unicode simple folding), `\s` is
// ASCII whitespace, `\b` is an ASCII word boundary, and the optional parts
// backtrack (every way of consuming them is tried).

fn space_run(s: &str) -> usize {
    s.bytes()
        .take_while(|b| matches!(b, b' ' | b'\t' | b'\n' | 0x0C | b'\r'))
        .count()
}

fn fold_eq(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase()) || a.to_uppercase().eq(b.to_uppercase())
}

/// Whether `s` starts with `lit` (case-insensitive per char) and the bytes of
/// `s` it covered. An empty `lit` matches with 0.
fn fold_prefix(s: &str, lit: &str) -> Option<usize> {
    let mut chars = s.char_indices();
    let mut used = 0;
    for l in lit.chars() {
        let (i, c) = chars.next()?;
        if !fold_eq(l, c) {
            return None;
        }
        used = i + c.len_utf8();
    }
    Some(used)
}

fn is_word_byte(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

/// Regexp `\b` at `pos`: exactly one of the bytes around it is a word byte.
fn word_boundary(text: &str, pos: usize) -> bool {
    let bytes = text.as_bytes();
    let before = pos > 0 && is_word_byte(bytes[pos - 1]);
    let after = pos < bytes.len() && is_word_byte(bytes[pos]);
    before != after
}

fn after_header(text: &str, header: &Regex, ssc: &str) -> bool {
    header.find_iter(text).any(|m| {
        let start = m.end();
        let end = start + space_run(&text[start..]);
        (start..=end)
            .rev()
            .any(|e| fold_prefix(&text[e..], ssc).is_some_and(|n| word_boundary(text, e + n)))
    })
}

/// The text names the formula `ssc` right after one of the two known headers.
fn mentions_ssc(text: &str, ssc: &str) -> bool {
    after_header(text, &FORMULA_NO_HEADER, ssc) || after_header(text, &SSC_BARE_HEADER, ssc)
}

/// How many bytes the optional quote `["']?` may consume at `pos`.
fn quote_offsets(text: &str, pos: usize) -> Vec<usize> {
    match text.as_bytes().get(pos) {
        Some(b'"' | b'\'') => vec![0, 1],
        _ => vec![0],
    }
}

/// The row contains `AjaxModalVerFormula(ssc)` with optional quotes and
/// whitespace around the SSC.
fn opens_formula(row: &str, ssc: &str) -> bool {
    OPEN_FORMULA_HEAD.find_iter(row).any(|m| {
        let start = m.end();
        let end = start + space_run(&row[start..]);
        (start..=end).rev().any(|e| {
            quote_offsets(row, e).into_iter().any(|q| {
                let Some(n) = fold_prefix(&row[e + q..], ssc) else {
                    return false;
                };
                let p = e + q + n;
                quote_offsets(row, p).into_iter().any(|q2| {
                    let r = p + q2;
                    let r = r + space_run(&row[r..]);
                    row.as_bytes().get(r) == Some(&b')')
                })
            })
        })
    })
}

// ----------------------------------------------------------------- parsers

/// Whether the first list row that opens the formula `ssc`
/// (`AjaxModalVerFormula(ssc)`) mentions `invoice`.
pub fn list_contains_formula(page: &str, ssc: &str, invoice: &str) -> bool {
    let inv = ALNUM_OUT
        .replace_all(&invoice.to_uppercase(), "")
        .into_owned();
    for row in rows(page) {
        if !opens_formula(&row, ssc) {
            continue;
        }
        let text = ALNUM_OUT
            .replace_all(&page_text(&row).to_uppercase(), "")
            .into_owned();
        return !inv.is_empty() && text.contains(&inv);
    }
    false
}

/// Reads the formula date and the detail row whose molecule best matches
/// `article` (the longest molecule contained in the article or containing
/// it). Rows need 6 cells with a numeric quantity.
pub fn parse_formula_detail(page: &str, ssc: &str, article: &str) -> Result<Formula, PageError> {
    let text = page_text(page);
    if !mentions_ssc(&text, ssc) {
        return Err(PageError(format!(
            "el detalle recibido no corresponde a la formula {ssc}"
        )));
    }
    let Some(date) = FECHA.captures(&text) else {
        return Err(PageError(
            "Medicar no informo la fecha de la formula".into(),
        ));
    };
    let target = normalize_name(article);
    let mut best: Option<(usize, Formula)> = None;
    for row in rows(page) {
        let c = cells(&row);
        if c.len() < 6 {
            continue;
        }
        let Ok(qty) = c[5].trim().replace(',', ".").parse::<Decimal>() else {
            continue;
        };
        let mol = normalize_name(&c[1]);
        if mol.is_empty() || !(target.contains(&mol) || mol.contains(&target)) {
            continue;
        }
        let score = mol.chars().count();
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((
                score,
                Formula {
                    molecula: c[1].trim().to_owned(),
                    articulo_medicar: c[2].trim().to_owned(),
                    estado_despacho: c[3].trim().to_owned(),
                    fecha_despacho: DISPATCH
                        .find(&c[4])
                        .map(|m| m.as_str().to_owned())
                        .unwrap_or_default(),
                    cantidad: qty,
                    ..Formula::default()
                },
            ));
        }
    }
    let Some((_, mut best)) = best else {
        return Err(PageError(
            "ninguna molecula de Medicar coincide con el articulo del convenio".into(),
        ));
    };
    if best.cantidad <= Decimal::ZERO {
        return Err(PageError(
            "la cantidad encontrada en Medicar no es valida".into(),
        ));
    }
    best.fecha_formula = date[1].to_owned();
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_picks_the_longest_molecule_with_accents_folded() {
        let page = "<p>SSC 55 &nbsp; Fecha: 2026-01-02</p><tr><td>1<td>Gasa<td>GASA SINT<td>OK<td>sin fecha<td>2</tr>\
                    <tr><td>2</td><td>Gasa esteril</td><td>GASA EST</td><td>OK</td><td>2026-01-03 07:00:00</td><td>4</td></tr>";
        let f = parse_formula_detail(page, "55", "GASA ESTÉRIL 10X10").expect("detail");
        assert_eq!(f.molecula, "Gasa esteril");
        assert_eq!(f.cantidad, Decimal::from(4));
        assert_eq!(f.fecha_formula, "2026-01-02");
        assert_eq!(f.fecha_despacho, "2026-01-03 07:00:00");

        let unclosed =
            "SSC 55 Fecha: 2026-01-02 <tr><td>1<td>Gasa<td>GASA SINT<td>OK<td>sin fecha<td>2";
        let f = parse_formula_detail(unclosed, "55", "GASA").expect("unclosed cells");
        assert_eq!(
            (f.molecula.as_str(), f.fecha_despacho.as_str(), f.cantidad),
            ("Gasa", "", Decimal::from(2))
        );
    }

    #[test]
    fn detail_rejections() {
        let page = "<p>SSC 55 Fecha: 2026-01-02</p><tr><td>1</td><td>Gasa</td><td>a</td><td>b</td><td>c</td><td>1</td></tr>";
        let err = |page: &str, ssc: &str| {
            parse_formula_detail(page, ssc, "GASA")
                .expect_err("rejected")
                .0
        };
        assert_eq!(
            err(page, "56"),
            "el detalle recibido no corresponde a la formula 56"
        );
        assert_eq!(
            err(
                "SSC 55 <tr><td>1</td><td>Gasa</td><td>a</td><td>b</td><td>c</td><td>1</td></tr>",
                "55"
            ),
            "Medicar no informo la fecha de la formula"
        );
        assert_eq!(
            err(
                "SSC 55 Fecha: 2026-01-02 <tr><td>1</td><td>Gasa</td><td>a</td><td>b</td><td>c</td><td>0</td></tr>",
                "55"
            ),
            "la cantidad encontrada en Medicar no es valida"
        );
        assert_eq!(
            parse_formula_detail(page, "55", "IBUPROFENO")
                .expect_err("no molecule")
                .0,
            "ninguna molecula de Medicar coincide con el articulo del convenio"
        );
    }

    #[test]
    fn list_matches_the_row_that_opens_the_formula() {
        let list = "<tr><td>FVS-1</td><td onclick=\"AjaxModalVerFormula( '9' )\"></td></tr>\
                    <tr><td>FVS-2</td><td onclick=\"AjaxModalVerFormula(8)\"></td></tr>";
        assert!(list_contains_formula(list, "9", "fvs1"));
        assert!(list_contains_formula(list, "8", "FVS-2"));
        assert!(!list_contains_formula(list, "9", "FVS2"));
        assert!(!list_contains_formula(list, "7", "FVS1"));
        assert!(!list_contains_formula(list, "9", ""));
    }

    #[test]
    fn windows_1252_bodies_are_decoded() {
        assert_eq!(decode_portal(b"Fecha:\xa0ni\xf1o"), "Fecha:\u{a0}niño");
        assert_eq!(decode_portal("niño".as_bytes()), "niño");
        assert_eq!(decode_portal(b"\x80 \x93x\x94"), "€ \u{201C}x\u{201D}");
    }

    #[test]
    fn entities_and_csrf() {
        assert_eq!(
            unescape_html("a&amp;b &aacute;&Ntilde; &#65;&#x42; &bogus; &"),
            "a&b áÑ AB &bogus; &"
        );
        assert_eq!(LATIN1_ENTITIES.len(), 96);
        assert_eq!(named_entity("ntilde"), Some('ñ'));
        assert_eq!(named_entity("yuml"), Some('\u{ff}'));
        assert_eq!(
            find_csrf(r#"<input type="hidden" value="tok&amp;sint" name="csrf_token">"#).as_deref(),
            Some("tok&sint")
        );
        assert_eq!(
            find_csrf(r#"<input name='csrf_token' value='abc'>"#).as_deref(),
            Some("abc")
        );
        assert_eq!(find_csrf("<form></form>"), None);
    }

    #[test]
    fn ssc_matchers_follow_the_go_regexps() {
        for (text, ssc, want) in [
            ("Formula SSC No: 123 Fecha", "123", true),
            ("SSC 123 Fecha", "123", true),
            ("SSC123 Fecha", "123", true),
            ("formula ssc no: abc12 x", "ABC12", true),
            ("SSC 1234", "123", false),
            ("SSC1234", "123", false),
            ("SSC 123-9", "123", true),
            ("SSC 123_9", "123", false),
            ("SSC 123a", "123", false),
            ("Formula SSC No: 123", "123", true),
            ("SSC a- b", "a-", false),
            ("SSC a-b", "a-", true),
            ("SSC axb", "a.b", false),
            ("SSC 12 SSC 1", "1", true),
            ("123", "123", false),
            ("Formula SSC No: 999", "123", false),
            ("Formula\nSSC\r\nNo:\t\x0c123", "123", true),
            ("SSC ÉX1", "éx1", true),
            ("ſſc 5", "5", true),
            ("SSC 123é", "123", true),
            ("SSC 1", "12345", false),
        ] {
            assert_eq!(
                mentions_ssc(text, ssc),
                want,
                "mentions_ssc({text:?}, {ssc:?})"
            );
        }
    }

    #[test]
    fn opens_formula_follows_the_go_regexp() {
        for (row, ssc, want) in [
            (r#"onclick="AjaxModalVerFormula(9)""#, "9", true),
            (r#"AjaxModalVerFormula("9")"#, "9", true),
            (r#"AjaxModalVerFormula('9')"#, "9", true),
            (r#"AjaxModalVerFormula( '9' )"#, "9", true),
            ("AjaxModalVerFormula  (9)", "9", true),
            ("ajaxmodalverformula(abc)", "ABC", true),
            ("AjaxModalVerFormula(91)", "9", false),
            ("AjaxModalVerFormula(9)", "91", false),
            ("AjaxModalVerFormula('9)", "9", true),
            ("AjaxModalVerFormula(9')", "9", true),
            ("AjaxModalVerFormula()", "", true),
            ("AjaxModalVerFormula(axb)", "a.b", false),
            ("AjaxModalVerFormula(1) AjaxModalVerFormula(2)", "2", true),
            ("<td>9</td>", "9", false),
            ("AjaxModalVerFormula(9", "9", false),
            ("AjaxModalVerFormula(ÉX)", "éx", true),
        ] {
            assert_eq!(
                opens_formula(row, ssc),
                want,
                "opens_formula({row:?}, {ssc:?})"
            );
        }
    }
}
