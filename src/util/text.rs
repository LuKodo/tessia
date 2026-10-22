//! Text normalization shared by lookups that must tolerate case, accents and
//! spacing (subplan allowlists, catalog headers).

/// Upper-cases, strips Spanish/Latin accents (`á`->`A`, `ñ`->`N`) and collapses
/// runs of whitespace into single spaces (port of Go `catalogHeader`).
///
/// ```
/// assert_eq!(tessia::util::text::normalize_key("  Evento  pos "), "EVENTO POS");
/// assert_eq!(tessia::util::text::normalize_key("Autorización"), "AUTORIZACION");
/// ```
pub fn normalize_key(text: &str) -> String {
    let stripped: String = text.chars().map(strip_accent).collect();
    stripped
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

fn strip_accent(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
        'Á' | 'À' | 'Ä' | 'Â' | 'Ã' | 'Å' => 'A',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'É' | 'È' | 'Ë' | 'Ê' => 'E',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'Í' | 'Ì' | 'Ï' | 'Î' => 'I',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
        'Ó' | 'Ò' | 'Ö' | 'Ô' | 'Õ' => 'O',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'Ú' | 'Ù' | 'Ü' | 'Û' => 'U',
        'ñ' => 'n',
        'Ñ' => 'N',
        'ç' => 'c',
        'Ç' => 'C',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_spacing_case_and_accents() {
        assert_eq!(
            normalize_key("Transferencia  de PENDIENTES\tcon autorización"),
            "TRANSFERENCIA DE PENDIENTES CON AUTORIZACION"
        );
        assert_eq!(normalize_key("niño"), "NINO");
        assert_eq!(normalize_key("   "), "");
    }
}
