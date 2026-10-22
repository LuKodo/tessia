//! Helvetica text metrics and WinAnsi encoding (the two standard fonts the
//! delivery page uses; they are not embedded in the PDF).

use super::helvetica::{HELVETICA, HELVETICA_BOLD};

/// The byte a character takes in a WinAnsi/Latin-1 string: characters above
/// U+00FF and the C1 controls (0x7F..=0x9F) become `?`.
pub fn win_ansi_byte(c: char) -> u8 {
    match u32::from(c) {
        code @ 0x20..=0x7E | code @ 0xA0..=0xFF => code as u8,
        _ => b'?',
    }
}

/// WinAnsi bytes of `text` (control characters such as newlines are dropped).
pub fn encode(text: &str) -> Vec<u8> {
    text.chars()
        .filter(|c| !c.is_control() || (0x80..=0x9F).contains(&u32::from(*c)))
        .map(win_ansi_byte)
        .collect()
}

/// Width in points of `text` in Helvetica (or Helvetica-Bold) at `size`.
pub fn text_width(text: &str, bold: bool, size: f64) -> f64 {
    let table = if bold { &HELVETICA_BOLD } else { &HELVETICA };
    let units: u32 = encode(text)
        .iter()
        .map(|b| u32::from(table[usize::from(*b) - 0x20]))
        .sum();
    f64::from(units) * size / 1000.0
}

/// Escapes bytes for a PDF literal string (non-ASCII as octal, so the content
/// stream stays 7-bit).
pub fn pdf_literal(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('(');
    for &b in bytes {
        match b {
            b'\\' | b'(' | b')' => {
                out.push('\\');
                out.push(char::from(b));
            }
            0x20..=0x7E => out.push(char::from(b)),
            _ => out.push_str(&format!("\\{b:03o}")),
        }
    }
    out.push(')');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_follow_the_adobe_metrics() {
        // "Hello": H=722 e=556 l=222 l=222 o=556 at 10 pt.
        assert!((text_width("Hello", false, 10.0) - 22.78).abs() < 1e-9);
        // Bold: H=722 e=556 l=278 l=278 o=611.
        assert!((text_width("Hello", true, 10.0) - 24.45).abs() < 1e-9);
        assert!((text_width("", false, 12.0)).abs() < 1e-12);
    }

    #[test]
    fn accents_are_one_latin1_byte_and_the_rest_is_a_question_mark() {
        assert_eq!(encode("añ¿"), vec![b'a', 0xF1, 0xBF]);
        assert_eq!(encode("€x\n"), vec![b'?', b'x']);
        assert!(text_width("ñ", false, 10.0) > 0.0);
    }

    #[test]
    fn literals_escape_delimiters_and_high_bytes() {
        assert_eq!(pdf_literal(b"a(b)\\"), "(a\\(b\\)\\\\)");
        assert_eq!(pdf_literal(&[0xF1]), "(\\361)");
    }
}
