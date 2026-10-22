//! Rust ports of the Java library calls the legacy RIPS screen relied on
//! (`Integer.parseInt`, `Double.parseDouble`, `String.trim`, `substring` on
//! UTF-16 units, the date helpers). Go source: `internal/rips/java.go`.
//!
//! Behavior follows the Go port, which is the authority; where Go and Java
//! differ (a double that overflows is an error here) Go wins.

use std::ops::RangeInclusive;

/// `String.trim()`: strips code points `<= U+0020` at both ends.
pub fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `Integer.parseInt`: optional sign, ASCII digits, `i32` range.
pub fn java_parse_int(s: &str) -> Option<i32> {
    let body = s.trim_start_matches(['+', '-']);
    if s.len() - body.len() > 1 || body.is_empty() {
        return None;
    }
    if !body.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i32>().ok()
}

/// `Double.parseDouble` for decimal input (trimmed, optional `f`/`d` suffix).
/// Hex, `NaN`, `Infinity` and values that overflow are rejected.
pub fn java_parse_double(s: &str) -> Option<f64> {
    let t = java_trim(s);
    if !is_java_double(t) {
        return None;
    }
    let t = t.trim_end_matches(['f', 'F', 'd', 'D']);
    let value: f64 = t.parse().ok()?;
    value.is_finite().then_some(value)
}

/// `^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?[fFdD]?$` with ASCII digits.
fn is_java_double(t: &str) -> bool {
    let b = t.as_bytes();
    let digits = |from: usize| b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut i = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    let int_digits = digits(i);
    i += int_digits;
    if int_digits > 0 {
        if b.get(i) == Some(&b'.') {
            i += 1 + digits(i + 1);
        }
    } else {
        if b.get(i) != Some(&b'.') {
            return false;
        }
        let frac = digits(i + 1);
        if frac == 0 {
            return false;
        }
        i += 1 + frac;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        i += usize::from(matches!(b.get(i), Some(b'+' | b'-')));
        let exp = digits(i);
        if exp == 0 {
            return false;
        }
        i += exp;
    }
    i += usize::from(matches!(b.get(i), Some(b'f' | b'F' | b'd' | b'D')));
    i == b.len()
}

/// The `(int)` cast of a double: truncation, saturating, `NaN` is 0.
pub fn java_double_to_int(f: f64) -> i32 {
    f as i32
}

/// `s.length() > n ? s.substring(0, n) : s` counting UTF-16 units. A cut in
/// the middle of a surrogate pair leaves U+FFFD (as the Go port does).
pub fn truncate_utf16(s: &str, n: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    if units.len() <= n {
        return s.to_owned();
    }
    String::from_utf16_lossy(&units[..n])
}

/// `strings.ToUpper`: simple (one rune to one rune) case mapping.
pub fn go_upper(s: &str) -> String {
    s.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) => u,
                _ => c,
            }
        })
        .collect()
}

/// Go's `%q` for the strings that appear in `DATOS_INVALIDOS` details.
pub fn go_quote(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c if c != ' ' && (c.is_whitespace() || c.is_control()) => {
                if (c as u32) <= 0xffff {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                } else {
                    out.push_str(&format!("\\U{:08x}", c as u32));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Lossy view of the first `n` bytes (Go slices bytes, Java slices chars; the
/// inputs are ASCII dates in practice).
fn byte_prefix(s: &str, n: usize) -> String {
    String::from_utf8_lossy(&s.as_bytes()[..n.min(s.len())]).into_owned()
}

/// `normalizarFechaXml`: a `yyyy-MM-dd` prefix is kept as is, otherwise the
/// first token is parsed as `d/MM/yyyy`, `d-MM-yyyy` or `yyyy/MM/dd` (Java
/// `ResolverStyle.SMART`) and printed ISO. Empty when nothing matches.
pub fn normalize_fecha_xml(v: &str) -> String {
    let t = java_trim(v);
    if t.is_empty() {
        return String::new();
    }
    let b = t.as_bytes();
    if b.len() >= 10 && b[4] == b'-' && b[7] == b'-' {
        return byte_prefix(t, 10);
    }
    let Some(first) = t.split_whitespace().next() else {
        return String::new();
    };
    for sep in ['/', '-'] {
        if let Some([d, m, y]) = date_parts(first, sep, 1..=2, 2..=2, 4..=4)
            && let Some(iso) = smart_date(y, m, d)
        {
            return iso;
        }
    }
    if let Some([y, m, d]) = date_parts(first, '/', 4..=4, 2..=2, 2..=2)
        && let Some(iso) = smart_date(y, m, d)
    {
        return iso;
    }
    String::new()
}

/// Splits `a<sep>b<sep>c` when every part is ASCII digits of the given lengths.
fn date_parts(
    s: &str,
    sep: char,
    a: RangeInclusive<usize>,
    b: RangeInclusive<usize>,
    c: RangeInclusive<usize>,
) -> Option<[&str; 3]> {
    let mut it = s.split(sep);
    let parts = [it.next()?, it.next()?, it.next()?];
    if it.next().is_some() {
        return None;
    }
    let fits = |part: &str, len: &RangeInclusive<usize>| {
        len.contains(&part.len()) && part.bytes().all(|x| x.is_ascii_digit())
    };
    (fits(parts[0], &a) && fits(parts[1], &b) && fits(parts[2], &c)).then_some(parts)
}

/// Java `IsoChronology.resolveYMD` in SMART mode: month 1..12, day 1..31, the
/// day clamped to the month length. Returns `yyyy-MM-dd`.
fn smart_date(y: &str, m: &str, d: &str) -> Option<String> {
    let year: u32 = y.parse().ok()?;
    let month: u32 = m.parse().ok()?;
    let day: u32 = d.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let day = day.min(month_length(year, month));
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

fn month_length(year: u32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 31,
    }
}

/// `"HH:mm"` after the first space (or `T`), `"0H:mm"` for a one-digit hour,
/// empty when absent.
pub fn extract_hour_minute(texto: &str) -> String {
    let v = java_trim(texto).replace('T', " ");
    let Some(i) = v.find(' ') else {
        return String::new();
    };
    if i + 1 >= v.len() {
        return String::new();
    }
    let h = java_trim(&v[i + 1..]);
    let b = h.as_bytes();
    if b.len() >= 5 && b[2] == b':' {
        byte_prefix(h, 5)
    } else if b.len() >= 4 && b[1] == b':' {
        format!("0{}", byte_prefix(h, 4))
    } else {
        String::new()
    }
}

/// `dd/MM/yyyy` becomes ISO; anything else is kept as given.
pub fn birth_date_iso(v: &str) -> String {
    if let Some([d, m, y]) = date_parts(v, '/', 2..=2, 2..=2, 4..=4)
        && let Some(iso) = smart_date(y, m, d)
    {
        return iso;
    }
    v.to_owned()
}

/// The Java replace chain of `normalizarTipoUsuario` (`EPS048` -> `01`, ...).
pub fn normalize_tipo_usuario(v: &str) -> String {
    let mut s = go_upper(java_trim(v));
    for (from, to) in [
        ("EPS048", "01"),
        ("EPSS48", "04"),
        ("ESS207", "04"),
        ("ESSC07", "01"),
        ("SUBSIDIADO", "04"),
        ("CONTRIBUTIVO", "01"),
    ] {
        s = s.replace(from, to);
    }
    s
}

/// `normalizarZona`: `R`/rural -> `01`, `U`/urban -> `02`, else upper-cased.
pub fn normalize_zona(v: &str) -> String {
    let z = go_upper(java_trim(v));
    if z == "R" || z.contains("RURAL") {
        "01".to_owned()
    } else if z == "U" || z.contains("URBAN") {
        "02".to_owned()
    } else {
        z
    }
}

/// `.replace(",00", "").replace(",", ".")`.
pub fn java_money(s: &str) -> String {
    s.replace(",00", "").replace(',', ".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_int_follows_integer_parse_int() {
        for (input, want) in [
            ("0", 0),
            ("15", 15),
            ("-3", -3),
            ("+4", 4),
            ("2147483647", 2147483647),
        ] {
            assert_eq!(java_parse_int(input), Some(want), "{input:?}");
        }
        for input in ["", " 1", "1.0", "2147483648", "+-1", "1e3", "-", "+"] {
            assert_eq!(java_parse_int(input), None, "{input:?}");
        }
    }

    #[test]
    fn parse_double_and_cast_follow_java() {
        for (input, want) in [
            ("10.9", 10),
            (" 3 ", 3),
            ("-2.7", -2),
            ("1e3", 1000),
            ("5d", 5),
            (".5", 0),
            ("3e10", 2147483647),
            ("1.", 1),
            ("+.5e1", 5),
            ("1.e2", 100),
        ] {
            let value = java_parse_double(input);
            assert_eq!(value.map(java_double_to_int), Some(want), "{input:?}");
        }
        for input in [
            "", "abc", "1,5", "NaN", "0x10", "1e", "1e+", ".", "-", "1dd", "1e999", "Infinity",
        ] {
            assert_eq!(java_parse_double(input), None, "{input:?}");
        }
    }

    #[test]
    fn normalize_fecha_xml_matches_the_go_table() {
        for (input, want) in [
            ("2026-09-01 10:22:33", "2026-09-01"),
            ("2026-09-01T10:22", "2026-09-01"),
            ("1/09/2026 10:00", "2026-09-01"),
            ("01/09/2026", "2026-09-01"),
            ("1-09-2026", "2026-09-01"),
            ("2026/09/01", "2026-09-01"),
            ("31/02/2026", "2026-02-28"),
            ("1/9/2026", ""),
            ("1/09-2026", ""),
            ("32/01/2026", ""),
            ("", ""),
            ("ayer", ""),
        ] {
            assert_eq!(normalize_fecha_xml(input), want, "{input:?}");
        }
    }

    #[test]
    fn small_helpers_match_the_go_tests() {
        assert_eq!(
            truncate_utf16("ÑANDÚ ABCDEFGHIJKLMNOPQRSTUVWXYZ", 30),
            "ÑANDÚ ABCDEFGHIJKLMNOPQRSTUVWX"
        );
        // A surrogate pair cut in half becomes U+FFFD, like Go.
        assert_eq!(truncate_utf16("ab😀", 3), "ab\u{fffd}");
        assert_eq!(truncate_utf16("ab😀", 4), "ab😀");
        assert_eq!(birth_date_iso("31/12/2000"), "2000-12-31");
        assert_eq!(birth_date_iso("2000-12-31"), "2000-12-31");
        for (input, want) in [
            ("EPS048", "01"),
            ("subsidiado", "04"),
            ("01", "01"),
            ("04", "04"),
        ] {
            assert_eq!(normalize_tipo_usuario(input), want, "{input:?}");
        }
        for (input, want) in [("R", "01"), ("U", "02"), ("urbana", "02"), ("x", "X")] {
            assert_eq!(normalize_zona(input), want, "{input:?}");
        }
        // Java's replace(",00", "") quirk.
        assert_eq!(java_money("1,005"), "15");
        assert_eq!(java_money("1500,75"), "1500.75");
    }

    #[test]
    fn hour_minute_extraction() {
        assert_eq!(extract_hour_minute("2026-09-01 10:22:33"), "10:22");
        assert_eq!(extract_hour_minute("2026-09-01T7:05"), "07:05");
        assert_eq!(extract_hour_minute("2026-09-01"), "");
        assert_eq!(extract_hour_minute("2026-09-01 "), "");
    }

    #[test]
    fn go_quote_matches_percent_q() {
        assert_eq!(go_quote("500 MG"), "\"500 MG\"");
        assert_eq!(go_quote("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
        assert_eq!(go_quote("\u{0}\u{a0}ñ"), "\"\\x00\\u00a0ñ\"");
    }
}
