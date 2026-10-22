//! Text helpers shared by the byte-for-byte XML edits.

use regex::{NoExpand, Regex};

/// Java's `\s` is `[ \t\n\x0B\f\r]`; Go's lacks `\x0B`, so patterns spell it out.
const WS: &str = r"[\t\n\x0B\x0C\r ]";

/// Compiles `pattern` case-insensitively with `.` matching newlines, expanding
/// `\s` to Java's whitespace class.
///
/// All patterns are compile-time constants exercised by the unit tests, so a
/// failure here is a programming error and aborts at first use.
pub(crate) fn re(pattern: &str) -> Regex {
    let full = format!("(?is){}", pattern.replace(r"\s", WS));
    match Regex::new(&full) {
        Ok(r) => r,
        Err(e) => panic!("fevxml: invalid built-in pattern {full:?}: {e}"),
    }
}

/// Java's `String.trim()`: strips every char <= U+0020.
pub(crate) fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Escapes `&`, `<` and `>` (not quotes), like the Java helper.
pub(crate) fn escape(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Mirrors `new String(bytes, UTF_8)` as the Go port does: every invalid byte
/// becomes one U+FFFD.
pub(crate) fn decode(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_owned(),
        Err(_) => {
            let mut out = String::with_capacity(b.len());
            let mut rest = b;
            while !rest.is_empty() {
                match std::str::from_utf8(rest) {
                    Ok(s) => {
                        out.push_str(s);
                        break;
                    }
                    Err(e) => {
                        let (valid, after) = rest.split_at(e.valid_up_to());
                        // `valid` is valid UTF-8 by construction of `valid_up_to`.
                        out.push_str(&String::from_utf8_lossy(valid));
                        out.push('\u{FFFD}');
                        rest = &after[1..];
                    }
                }
            }
            out
        }
    }
}

/// Removes every match of `r` from `s`.
pub(crate) fn remove_all(r: &Regex, s: &str) -> String {
    r.replace_all(s, NoExpand("")).into_owned()
}

/// Replaces the first match of `r` in `s` with `f(captures)`; `None` when there
/// is no match.
pub(crate) fn replace_first<F>(r: &Regex, s: &str, f: F) -> Option<String>
where
    F: FnOnce(&regex::Captures<'_>) -> String,
{
    let caps = r.captures(s)?;
    let whole = caps.get(0)?;
    let mut out = String::with_capacity(s.len());
    out.push_str(&s[..whole.start()]);
    out.push_str(&f(&caps));
    out.push_str(&s[whole.end()..]);
    Some(out)
}

/// Text of capture group `i`, empty when the group did not participate.
pub(crate) fn group<'a>(caps: &regex::Captures<'a>, i: usize) -> &'a str {
    caps.get(i).map_or("", |m| m.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_trim_strips_control_chars() {
        assert_eq!(java_trim("\u{0B} a b\t\n"), "a b");
    }

    #[test]
    fn decode_replaces_each_invalid_byte() {
        assert_eq!(decode(b"a\xE2\x82b"), "a\u{FFFD}\u{FFFD}b");
        assert_eq!(decode("ñ".as_bytes()), "ñ");
    }

    #[test]
    fn escape_leaves_quotes() {
        assert_eq!(escape("a&<>\"'"), "a&amp;&lt;&gt;\"'");
    }
}
