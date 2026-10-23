//! An order-preserving JSON reader (Go `cuv.parseOrdered`).
//!
//! The sweeps rewrite SISPRO's TXT keeping the member order and the number
//! literals exactly as written, so `serde_json::Value` (which normalizes
//! numbers) cannot be used. The syntax rules are Go's `encoding/json`: strict
//! RFC 8259, a repeated member replaces the value in place, invalid UTF-8 in a
//! string becomes U+FFFD and lone surrogate escapes become U+FFFD.

use crate::util::gson::Json;

/// Nesting deeper than this is refused (Go allows 10 000; no SISPRO response
/// comes close and a smaller limit keeps the recursion safe on small stacks).
const MAX_DEPTH: usize = 1_000;

/// Parses a whole document; anything after the value (but whitespace) is an
/// error, as in Go.
pub fn parse_ordered(input: &[u8]) -> Result<Json, String> {
    let mut parser = Parser { src: input, pos: 0 };
    parser.skip_ws();
    let value = parser.value(0)?;
    parser.skip_ws();
    if parser.pos != input.len() {
        return Err("cuv: datos despues del JSON".into());
    }
    Ok(value)
}

/// `JsonObject.get`: the value of a member, if present.
pub fn get<'a>(members: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    members.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// `JsonObject.add`: an existing member keeps its position.
pub fn set(members: &mut Vec<(String, Json)>, key: &str, value: Json) {
    match members.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value,
        None => members.push((key.to_owned(), value)),
    }
}

/// `JsonElement.getAsString` for primitives (numbers keep their literal).
pub fn primitive_string(value: &Json) -> Result<String, String> {
    match value {
        Json::String(s) => Ok(s.clone()),
        Json::Number(literal) => Ok(literal.clone()),
        Json::Bool(b) => Ok(b.to_string()),
        _ => Err("se esperaba un valor primitivo".into()),
    }
}

/// Java's `String.trim()`: removes every character up to U+0020.
pub fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("cuv: se esperaba '{}'", byte as char))
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.src[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err("cuv: literal invalido".into())
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err("cuv: JSON demasiado anidado".into());
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Json::String),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err("cuv: caracter inesperado".into()),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.pos += 1;
        let mut members: Vec<(String, Json)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err("cuv: se esperaba el nombre de un miembro".into());
            }
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.value(depth + 1)?;
            set(&mut members, &key, value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err("cuv: se esperaba ',' o '}'".into()),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err("cuv: se esperaba ',' o ']'".into()),
            }
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err("cuv: numero invalido".into()),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err("cuv: numero invalido".into());
            }
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err("cuv: numero invalido".into());
            }
            self.digits();
        }
        let literal = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
        Ok(Json::Number(literal))
    }

    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .src
            .get(self.pos..self.pos + 4)
            .ok_or("cuv: escape \\u incompleto")?;
        let text = std::str::from_utf8(digits).map_err(|_| "cuv: escape \\u invalido")?;
        let value = u32::from_str_radix(text, 16).map_err(|_| "cuv: escape \\u invalido")?;
        self.pos += 4;
        Ok(value)
    }

    /// A `\uXXXX` escape (the `\u` is already consumed), pairing surrogates.
    fn unicode_escape(&mut self) -> Result<char, String> {
        let first = self.hex4()?;
        if (0xD800..0xDC00).contains(&first) && self.src[self.pos..].starts_with(b"\\u") {
            let save = self.pos;
            self.pos += 2;
            let second = self.hex4()?;
            if (0xDC00..0xE000).contains(&second) {
                let code = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
                return Ok(char::from_u32(code).unwrap_or('\u{FFFD}'));
            }
            self.pos = save;
        }
        Ok(char::from_u32(first).unwrap_or('\u{FFFD}'))
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut raw: Vec<u8> = Vec::new();
        loop {
            let byte = self.peek().ok_or("cuv: texto sin cerrar")?;
            self.pos += 1;
            match byte {
                b'"' => return Ok(String::from_utf8_lossy(&raw).into_owned()),
                b'\\' => {
                    let escape = self.peek().ok_or("cuv: texto sin cerrar")?;
                    self.pos += 1;
                    let ch = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        _ => return Err("cuv: escape invalido".into()),
                    };
                    let mut buf = [0u8; 4];
                    raw.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                0..=0x1f => return Err("cuv: caracter de control en un texto".into()),
                other => raw.push(other),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Json, String> {
        parse_ordered(s.as_bytes())
    }

    #[test]
    fn keeps_member_order_number_literals_and_replaces_duplicates_in_place() {
        let doc = parse(r#"{"b": 1.50, "a": [1e3, -0, null], "b": 2}"#).expect("parse");
        let Json::Object(members) = doc else {
            panic!("object expected")
        };
        let keys: Vec<&str> = members.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["b", "a"]);
        assert_eq!(members[0].1, Json::Number("2".into()));
        assert_eq!(
            members[1].1,
            Json::Array(vec![
                Json::Number("1e3".into()),
                Json::Number("-0".into()),
                Json::Null
            ])
        );
    }

    #[test]
    fn follows_the_go_syntax_rules() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "1.",
            ".5",
            "+1",
            "'a'",
            "{\"a\" 1}",
            "[1] x",
            "\"\u{1}\"",
            "tru",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} must be rejected");
        }
        assert_eq!(parse(" \n[ ]\t").expect("ws"), Json::Array(vec![]));
        assert_eq!(parse("-0.5E+2").expect("n"), Json::Number("-0.5E+2".into()));
    }

    #[test]
    fn decodes_escapes_surrogates_and_invalid_utf8() {
        assert_eq!(
            parse(r#""a\nbñ😀\/""#).expect("s"),
            Json::String("a\nbñ😀/".into())
        );
        assert_eq!(
            parse(r#""\ud83dx""#).expect("lone"),
            Json::String("\u{FFFD}x".into())
        );
        assert_eq!(
            parse_ordered(b"\"a\xffb\"").expect("bytes"),
            Json::String("a\u{FFFD}b".into())
        );
    }

    #[test]
    fn deep_nesting_is_refused_instead_of_overflowing_the_stack() {
        let deep = "[".repeat(MAX_DEPTH + 5);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn helpers_match_the_gson_object_model() {
        let mut members = vec![("a".to_owned(), Json::Bool(true))];
        set(&mut members, "b", Json::Null);
        set(&mut members, "a", Json::Int(1));
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].1, Json::Int(1));
        assert_eq!(get(&members, "b"), Some(&Json::Null));
        assert_eq!(
            primitive_string(&Json::Number("7.0".into())).expect("n"),
            "7.0"
        );
        assert_eq!(primitive_string(&Json::Bool(false)).expect("b"), "false");
        assert!(primitive_string(&Json::Null).is_err());
        assert_eq!(java_trim("\u{1} x\n"), "x");
    }
}
