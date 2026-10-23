//! Gson-compatible JSON writer.
//!
//! The Java tool serialized its JSON artifacts (RIPS, CUV payloads) with Gson
//! and downstream systems compare bytes, so this writer reproduces Gson's
//! `JsonWriter` rules:
//!
//! * pretty printing uses a two-space indent and `"key": value`;
//! * object members keep insertion order (nothing is sorted);
//! * `null` members are dropped unless [`GsonOptions::serialize_nulls`];
//!   `null` array elements are always written;
//! * integers print as plain decimals, floats like `Double.toString`
//!   (`1.0`, `1.0E7`, `1.5E-5`), and [`Json::Number`] is written verbatim
//!   (the equivalent of a Java `BigDecimal`);
//! * strings escape `"` `\` and control characters, plus U+2028/U+2029;
//!   `< > & = '` are escaped as `<`... only when
//!   [`GsonOptions::html_escaping`] is set (Gson's default, disabled here
//!   because the Java tool called `disableHtmlEscaping()`).
//!
//! ```
//! use tessia::util::gson::{Json, GsonOptions, to_string};
//!
//! let doc = Json::object([
//!     ("factura", Json::from("FEV1")),
//!     ("total", Json::Number("10.50".into())),
//!     ("nota", Json::Null),
//! ]);
//! let text = to_string(&doc, &GsonOptions::default());
//! assert_eq!(text, "{\n  \"factura\": \"FEV1\",\n  \"total\": 10.50\n}");
//! ```

use std::fmt::Write as _;

/// A JSON tree whose objects keep member order.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Double(f64),
    /// A pre-formatted number written verbatim (must be a valid JSON number).
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Builds an object from `(key, value)` pairs, keeping their order.
    pub fn object<K, I>(members: I) -> Json
    where
        K: Into<String>,
        I: IntoIterator<Item = (K, Json)>,
    {
        Json::Object(members.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// Builds an array.
    pub fn array<I: IntoIterator<Item = Json>>(items: I) -> Json {
        Json::Array(items.into_iter().collect())
    }

    /// A decimal amount written verbatim, like a Java `BigDecimal`.
    pub fn decimal(value: rust_decimal::Decimal) -> Json {
        Json::Number(value.to_string())
    }
}

impl From<&str> for Json {
    fn from(value: &str) -> Self {
        Json::String(value.to_owned())
    }
}

impl From<String> for Json {
    fn from(value: String) -> Self {
        Json::String(value)
    }
}

impl From<bool> for Json {
    fn from(value: bool) -> Self {
        Json::Bool(value)
    }
}

impl From<i64> for Json {
    fn from(value: i64) -> Self {
        Json::Int(value)
    }
}

impl From<f64> for Json {
    fn from(value: f64) -> Self {
        Json::Double(value)
    }
}

impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(value: Option<T>) -> Self {
        value.map_or(Json::Null, Into::into)
    }
}

impl From<&serde_json::Value> for Json {
    fn from(value: &serde_json::Value) -> Self {
        use serde_json::Value;
        match value {
            Value::Null => Json::Null,
            Value::Bool(b) => Json::Bool(*b),
            Value::Number(n) => n
                .as_i64()
                .map(Json::Int)
                .or_else(|| n.as_u64().map(Json::UInt))
                .or_else(|| n.as_f64().map(Json::Double))
                .unwrap_or(Json::Null),
            Value::String(s) => Json::String(s.clone()),
            Value::Array(items) => Json::Array(items.iter().map(Json::from).collect()),
            Value::Object(map) => Json::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), Json::from(v)))
                    .collect(),
            ),
        }
    }
}

/// Writer options (the `GsonBuilder` switches).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GsonOptions {
    /// `serializeNulls()`: write `"key": null` members.
    pub serialize_nulls: bool,
    /// Escape `< > & = '` (Gson's default; the Java tool disabled it).
    pub html_escaping: bool,
    /// `setPrettyPrinting()`: two-space indent.
    pub pretty: bool,
}

impl Default for GsonOptions {
    /// Pretty, no HTML escaping, nulls dropped: what the Java tool produced.
    fn default() -> Self {
        Self {
            serialize_nulls: false,
            html_escaping: false,
            pretty: true,
        }
    }
}

/// Serializes `value`.
pub fn to_string(value: &Json, options: &GsonOptions) -> String {
    let mut out = String::new();
    write_value(&mut out, value, options, 0);
    out
}

/// Serializes any `serde::Serialize` value (member order of structs, or of a
/// `serde_json::Map` with `preserve_order`, is kept).
pub fn to_string_of<T: serde::Serialize>(
    value: &T,
    options: &GsonOptions,
) -> serde_json::Result<String> {
    let tree = serde_json::to_value(value)?;
    Ok(to_string(&Json::from(&tree), options))
}

fn newline(out: &mut String, options: &GsonOptions, depth: usize) {
    if options.pretty {
        out.push('\n');
        for _ in 0..depth {
            out.push_str("  ");
        }
    }
}

fn write_value(out: &mut String, value: &Json, options: &GsonOptions, depth: usize) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Int(n) => {
            let _ = write!(out, "{n}");
        }
        Json::UInt(n) => {
            let _ = write!(out, "{n}");
        }
        Json::Double(d) => out.push_str(&java_double(*d)),
        Json::Number(text) => out.push_str(text),
        Json::String(text) => write_string(out, text, options.html_escaping),
        Json::Array(items) => write_array(out, items, options, depth),
        Json::Object(members) => write_object(out, members, options, depth),
    }
}

fn write_array(out: &mut String, items: &[Json], options: &GsonOptions, depth: usize) {
    if items.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        newline(out, options, depth + 1);
        write_value(out, item, options, depth + 1);
    }
    newline(out, options, depth);
    out.push(']');
}

fn write_object(out: &mut String, members: &[(String, Json)], options: &GsonOptions, depth: usize) {
    let kept: Vec<&(String, Json)> = members
        .iter()
        .filter(|(_, v)| options.serialize_nulls || *v != Json::Null)
        .collect();
    if kept.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push('{');
    for (i, (key, value)) in kept.into_iter().map(|m| (&m.0, &m.1)).enumerate() {
        if i > 0 {
            out.push(',');
        }
        newline(out, options, depth + 1);
        write_string(out, key, options.html_escaping);
        out.push_str(if options.pretty { ": " } else { ":" });
        write_value(out, value, options, depth + 1);
    }
    newline(out, options, depth);
    out.push('}');
}

fn write_string(out: &mut String, text: &str, html: bool) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            '<' if html => out.push_str("\\u003c"),
            '>' if html => out.push_str("\\u003e"),
            '&' if html => out.push_str("\\u0026"),
            '=' if html => out.push_str("\\u003d"),
            '\'' if html => out.push_str("\\u0027"),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Formats like Java's `Double.toString`: plain notation in `[1e-3, 1e7)`
/// with at least one fractional digit, otherwise `d.dddE<exp>`.
pub fn java_double(value: f64) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value < 0.0 {
            "-Infinity".into()
        } else {
            "Infinity".into()
        };
    }
    let sign = if value < 0.0 { "-" } else { "" };
    let abs = value.abs();
    let scientific = format!("{abs:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    if (1e-3..1e7).contains(&abs) {
        if exponent >= 0 {
            let point = exponent as usize + 1;
            let padded = format!("{digits:0<point$}");
            let (int, frac) = padded.split_at(point);
            let frac = if frac.is_empty() { "0" } else { frac };
            format!("{sign}{int}.{frac}")
        } else {
            let zeros = "0".repeat((-exponent - 1) as usize);
            format!("{sign}0.{zeros}{digits}")
        }
    } else {
        let (first, rest) = digits.split_at(1);
        let rest = if rest.is_empty() { "0" } else { rest };
        format!("{sign}{first}.{rest}E{exponent}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compact() -> GsonOptions {
        GsonOptions {
            pretty: false,
            ..GsonOptions::default()
        }
    }

    #[test]
    fn pretty_output_uses_two_spaces_and_keeps_member_order() {
        let doc = Json::object([
            ("z", Json::Int(1)),
            ("a", Json::array([Json::Int(1), Json::Int(2)])),
            ("o", Json::object([("k", Json::from("v"))])),
        ]);
        let expected =
            "{\n  \"z\": 1,\n  \"a\": [\n    1,\n    2\n  ],\n  \"o\": {\n    \"k\": \"v\"\n  }\n}";
        assert_eq!(to_string(&doc, &GsonOptions::default()), expected);
    }

    #[test]
    fn empty_containers_stay_inline() {
        let doc = Json::object([("a", Json::array([])), ("o", Json::Object(vec![]))]);
        assert_eq!(
            to_string(&doc, &GsonOptions::default()),
            "{\n  \"a\": [],\n  \"o\": {}\n}"
        );
    }

    #[test]
    fn null_members_are_dropped_or_written_by_option() {
        let doc = Json::object([("a", Json::Null), ("b", Json::Int(1))]);
        assert_eq!(to_string(&doc, &compact()), "{\"b\":1}");
        let with_nulls = GsonOptions {
            serialize_nulls: true,
            ..compact()
        };
        assert_eq!(to_string(&doc, &with_nulls), "{\"a\":null,\"b\":1}");
    }

    #[test]
    fn null_array_elements_are_always_written() {
        let doc = Json::array([Json::Null, Json::Int(1)]);
        assert_eq!(to_string(&doc, &compact()), "[null,1]");
    }

    #[test]
    fn html_characters_are_not_escaped_by_default() {
        let doc = Json::from("a=b <c> & 'd'");
        assert_eq!(to_string(&doc, &compact()), "\"a=b <c> & 'd'\"");
        let escaped = GsonOptions {
            html_escaping: true,
            ..compact()
        };
        assert_eq!(
            to_string(&doc, &escaped),
            "\"a\\u003db \\u003cc\\u003e \\u0026 \\u0027d\\u0027\""
        );
    }

    #[test]
    fn control_and_separator_characters_are_escaped() {
        let doc = Json::from("a\"\\\n\t\u{1}\u{2028}é");
        assert_eq!(
            to_string(&doc, &compact()),
            "\"a\\\"\\\\\\n\\t\\u0001\\u2028é\""
        );
    }

    #[test]
    fn numbers_print_like_java() {
        assert_eq!(java_double(1.0), "1.0");
        assert_eq!(java_double(1.5), "1.5");
        assert_eq!(java_double(100.0), "100.0");
        assert_eq!(java_double(1.0e7), "1.0E7");
        assert_eq!(java_double(12345678.9), "1.23456789E7");
        assert_eq!(java_double(0.001), "0.001");
        assert_eq!(java_double(0.00015), "1.5E-4");
        assert_eq!(java_double(1.5e-5), "1.5E-5");
        assert_eq!(java_double(-2.5), "-2.5");
        assert_eq!(java_double(0.0), "0.0");
        assert_eq!(to_string(&Json::Int(-7), &compact()), "-7");
        assert_eq!(
            to_string(&Json::Number("10.50".into()), &compact()),
            "10.50"
        );
    }

    #[test]
    fn serde_values_keep_their_member_order() {
        #[derive(serde::Serialize)]
        struct Row {
            zeta: u32,
            alpha: Option<String>,
            beta: String,
        }
        let row = Row {
            zeta: 1,
            alpha: None,
            beta: "x".into(),
        };
        assert_eq!(
            to_string_of(&row, &compact()).expect("serialize"),
            "{\"zeta\":1,\"beta\":\"x\"}"
        );
    }
}
