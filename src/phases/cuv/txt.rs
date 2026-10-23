//! `radicacion.TXTHasCUV`: does a CUV TXT carry a valid CUV?
//!
//! Radicación step 2 reads the same files; the rule is copied here so this
//! phase does not depend on that one.

use serde_json::Value;

/// True when the TXT is JSON holding a valid CUV (`CodigoUnicoValidacion`,
/// `CodigoUnicoValidacionToShow` or `CUV`, at any depth), or states that the CUV
/// already exists or was validated.
pub fn txt_has_cuv(content: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(content) else {
        return false;
    };
    if find_cuv(&value).is_some() {
        return true;
    }
    let text = String::from_utf8_lossy(content).to_uppercase();
    text.contains("CUV")
        && ["YA SE ENCUENTRA", "YA EXISTE", "VALIDADO"]
            .iter()
            .any(|needle| text.contains(needle))
}

fn is_cuv_key(key: &str) -> bool {
    [
        "CodigoUnicoValidacion",
        "CodigoUnicoValidacionToShow",
        "CUV",
    ]
    .iter()
    .any(|name| key.eq_ignore_ascii_case(name))
}

fn valid_cuv(text: &str) -> bool {
    let trimmed = text.trim();
    let upper = trimmed.to_uppercase();
    !trimmed.is_empty() && trimmed != "-" && upper != "NULL" && !upper.contains("NO APLICA")
}

/// `jsonText`: the text of a value (compact JSON for objects and lists).
fn json_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn find_cuv(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                if is_cuv_key(key) {
                    let text = json_text(inner);
                    if valid_cuv(&text) {
                        return Some(text.trim().to_owned());
                    }
                }
            }
            map.values().find_map(find_cuv)
        }
        Value::Array(items) => items.iter().find_map(find_cuv),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_valid_cuv_at_any_depth_and_key_case() {
        assert!(txt_has_cuv(br#"{"CodigoUnicoValidacion":"abc"}"#));
        assert!(txt_has_cuv(br#"{"x":[{"y":{"cuv":" abc "}}]}"#));
        assert!(txt_has_cuv(br#"{"CodigoUnicoValidacionToShow":12}"#));
    }

    #[test]
    fn placeholders_are_not_a_cuv() {
        for body in [
            &br#"{"CodigoUnicoValidacion":null}"#[..],
            br#"{"CodigoUnicoValidacion":""}"#,
            br#"{"CodigoUnicoValidacion":" - "}"#,
            br#"{"CodigoUnicoValidacion":"null"}"#,
            br#"{"CodigoUnicoValidacion":"No Aplica"}"#,
            br#"{"ResultadosValidacion":[]}"#,
            b"not json",
            b"",
        ] {
            assert!(!txt_has_cuv(body), "{:?}", String::from_utf8_lossy(body));
        }
    }

    #[test]
    fn a_text_saying_the_cuv_already_exists_counts() {
        assert!(txt_has_cuv(br#"{"mensaje":"El CUV ya existe"}"#));
        assert!(txt_has_cuv(br#"{"mensaje":"CUV ya validado"}"#));
        assert!(!txt_has_cuv(br#"{"mensaje":"ya existe"}"#));
    }
}
