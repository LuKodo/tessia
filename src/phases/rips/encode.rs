//! Exact Gson bytes of the RIPS document. Go source: `internal/rips/encode.go`.

use super::java::{go_upper, java_trim};
use super::model::Rips;
use crate::util::gson::{GsonOptions, to_string_of};

/// The Gson of `ImprimirJSON`:
/// `new GsonBuilder().serializeNulls().disableHtmlEscaping().setPrettyPrinting()`.
pub const GSON_OPTIONS: GsonOptions = GsonOptions {
    serialize_nulls: true,
    html_escaping: false,
    pretty: true,
};

/// Returns the exact text Java wrote to `<F>.json` (UTF-8, no trailing
/// newline). It first applies the changes `ImprimirJSON` made while copying
/// the model into its `LinkedHashMap` ([`normalize_for_output`]); `rips` is not
/// modified.
///
/// Java then ran three `replace()` calls meant to drop null service lists;
/// they never match this output, so they are not reproduced.
pub fn encode(rips: &Rips) -> anyhow::Result<String> {
    let mut copy = rips.clone();
    normalize_for_output(&mut copy);
    Ok(to_string_of(&copy, &GSON_OPTIONS)?)
}

/// The map-building step of `ImprimirJSON`:
///
/// * `idMIPRES` goes through `normalizarIdMIPRES` (null when blank, SSC or X);
/// * medicine diagnosis codes are upper-cased;
/// * `(` and `)` are removed from `nomTecnologiaSalud`;
/// * `unidadMinDispensa` above 99 is written as 99.
pub fn normalize_for_output(rips: &mut Rips) {
    for user in &mut rips.usuarios {
        for m in user.servicios.medicamentos.iter_mut().flatten() {
            m.id_mipres = normalize_id_mipres_opt(m.id_mipres.as_deref());
            m.cod_diagnostico_principal = go_upper(&m.cod_diagnostico_principal);
            m.cod_diagnostico_relacionado = go_upper(&m.cod_diagnostico_relacionado);
            m.nom_tecnologia_salud = strip_parens(&m.nom_tecnologia_salud);
            m.unidad_min_dispensa = m.unidad_min_dispensa.min(99);
        }
        for o in user.servicios.otros_servicios.iter_mut().flatten() {
            o.id_mipres = normalize_id_mipres_opt(o.id_mipres.as_deref());
            o.nom_tecnologia_salud = strip_parens(&o.nom_tecnologia_salud);
        }
    }
}

fn strip_parens(s: &str) -> String {
    s.replace(['(', ')'], "")
}

fn normalize_id_mipres_opt(value: Option<&str>) -> Option<String> {
    value.and_then(normalize_id_mipres)
}

/// `normalizarIdMIPRES`: the trimmed value, or `None` when it is blank or
/// contains `SSC` or `X` (case-insensitive).
pub fn normalize_id_mipres(s: &str) -> Option<String> {
    let v = java_trim(s);
    if v.is_empty() {
        return None;
    }
    let upper = go_upper(v);
    if upper.contains("SSC") || upper.contains('X') {
        return None;
    }
    Some(v.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_mipres_is_trimmed_and_dropped_for_ssc_or_x() {
        for (input, want) in [
            (" 12 ", Some("12")),
            ("", None),
            ("   ", None),
            ("ssc1", None),
            ("1x", None),
            ("20260901", Some("20260901")),
        ] {
            assert_eq!(normalize_id_mipres(input).as_deref(), want, "{input:?}");
        }
    }
}
