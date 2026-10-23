//! Minimal `.env` parser: `KEY=VALUE` lines, blank lines, `#` comments, an
//! optional `export ` prefix and optional single or double quotes around the
//! value (no escapes, no interpolation). Errors mention line numbers and key
//! names, never values.

use anyhow::{Result, bail};

/// Parses `content` into `(name, value)` pairs in file order.
pub fn parse(content: &str) -> Result<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (index, raw) in content.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            bail!("línea {line_no}: se esperaba CLAVE=VALOR");
        };
        let key = key.trim();
        if key.is_empty() {
            bail!("línea {line_no}: clave vacía");
        }
        let mut value = value.trim();
        if let Some(quote) = value.chars().next().filter(|c| *c == '"' || *c == '\'') {
            if value.len() < 2 || !value.ends_with(quote) {
                bail!("línea {line_no}: comilla sin cerrar en {key}");
            }
            value = &value[1..value.len() - 1];
        }
        if out.iter().any(|(k, _)| k == key) {
            bail!("línea {line_no}: clave duplicada {key}");
        }
        out.push((key.to_owned(), value.to_owned()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quotes_comments_and_export() {
        let pairs = parse("# c\n\nA=1\nexport B = \"two words\"\nC='x'\r\nD=\n").expect("parse");
        assert_eq!(
            pairs,
            [("A", "1"), ("B", "two words"), ("C", "x"), ("D", "")]
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
        );
    }

    #[test]
    fn errors_name_the_line_and_key_but_not_the_value() {
        let err = parse("A=1\nB\n").expect_err("no equals");
        assert!(err.to_string().contains("línea 2"));
        let err = parse("A=\"secret\nB=2").expect_err("quote");
        assert!(err.to_string().contains("línea 1") && !err.to_string().contains("secret"));
        assert!(
            parse("A=1\nA=2")
                .expect_err("dup")
                .to_string()
                .contains("duplicada A")
        );
        assert!(parse("=1").is_err());
    }
}
