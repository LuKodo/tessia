//! Adapter from the Redis Nota Técnica (`NtRow`) to the catalog the rules use.
//!
//! Go read an xlsx workbook (`internal/convenio/catalog.go`); S6 replaces it
//! with the read-only Redis collection. Mapping decisions (all in this
//! file, one place to change):
//!
//! | Go catalog field | `NtRow` field |
//! |---|---|
//! | key part `COD GEN` (`normCode`) | `cod_gen_siesa` |
//! | key part `CUM` and billing CUM | `cum_facturar` |
//! | product type 714/799 | `tipo_producto` (else `clasificacion` when that is exactly 714/799); an empty value keeps Go's default rule (dash in the CUM or more than 14 characters = 714, else 799) |
//! | agreed price | `precio_contrato`, else `precio_final` (Go: the first column whose header holds PRECIO CONTRATO or PRECIO FINAL); `precio_maximo` is a ceiling, not the agreed price, and is ignored. No price = `0`, which skips the price rule |
//! | pharmaceutical form | `ffm_rips` |
//! | concentration | `upr_rips` (see the open question below) |
//! | measure unit | `umm_rips` |
//!
//! **Open question (needs a real NT row to confirm):** the three RIPS fields
//! are `ffm_rips`, `umm_rips` and `upr_rips`; Go has form, concentration and
//! unit. `ffm` (forma farmacéutica) and `umm` (unidad de medida) are
//! unambiguous, so concentration takes `upr_rips`. A row may override it with
//! an explicit extra key (`concentracion`, `concentracion_rips`,
//! `concentracion_medicamento_rips`).
//!
//! **Several rows per key.** The catalog key is `normCode(cod_gen_siesa) |
//! normCode(cum_facturar)`. Go's rule "the last duplicate wins" is kept
//! because later rows of a source sheet are corrections: the rows returned by
//! `NtCatalog::lookup` (ordered by id, `<id>#N` variants included) are
//! inserted in that order, so the last eligible one stays. A row is
//! **eligible** only when its `estado` is empty or `VALIDADO` (accent/case
//! insensitive); rows in any other state never authorize billing, so a line
//! that only matches ineligible rows reads as "No cruza nota tecnica".
//! Rows without CUM or generic code are skipped (as Go skips them).
//!
//! **Lookup.** Only `cum_facturar` is indexed in Redis, so the catalog is
//! built per run from the distinct CUMs of the CSV: one `lookup` each (the
//! trimmed value, and its `normCode` form when different), a few threads at a
//! time. The NT version is read once ([`resolve_version`]) and pinned.

use std::collections::BTreeSet;

use anyhow::{Result, anyhow};

use super::amount::price_text;
use super::catalog::{Catalog, Item, normalize_code, normalize_product_type};
use super::csv::Line;
use crate::nt::{NtCatalog, NtRow};
use crate::util::par;
use crate::util::text::normalize_key;

/// Only rows in this state authorize billing (empty counts as valid).
const VALID_ESTADO: &str = "VALIDADO";

/// Concurrent Redis lookups while building the catalog.
const LOOKUP_WORKERS: usize = 8;

/// Extra keys that may carry the concentration explicitly.
const CONCENTRATION_KEYS: [&str; 3] = [
    "concentracion",
    "concentracion_rips",
    "concentracion_medicamento_rips",
];

/// The version to use: the one pinned by an earlier preparation, else the
/// current one.
pub fn resolve_version(nt: &dyn NtCatalog, pinned: Option<&str>) -> Result<String> {
    if let Some(version) = pinned.filter(|v| !v.trim().is_empty()) {
        return Ok(version.to_owned());
    }
    nt.current_version()?.ok_or_else(|| {
        anyhow!("la Nota Técnica no tiene ninguna versión en Redis (nt:actual vacío)")
    })
}

/// True when the row may authorize billing.
pub fn is_eligible(row: &NtRow) -> bool {
    match row.estado.as_deref().map(normalize_key) {
        None => true,
        Some(estado) => estado.is_empty() || estado == VALID_ESTADO,
    }
}

/// Maps one row to a catalog item; `None` when it lacks CUM or generic code.
pub fn item_from_row(row: &NtRow) -> Option<Item> {
    let cum = row.cum_facturar.as_deref().map(str::trim).unwrap_or("");
    let code = row.cod_gen_siesa.as_deref().map(str::trim).unwrap_or("");
    if cum.is_empty() || code.is_empty() {
        return None;
    }
    Some(Item {
        cum: cum.to_owned(),
        product_type: normalize_product_type(&product_type_of(row), cum),
        generic_code: code.to_owned(),
        price: row
            .precio_contrato
            .or(row.precio_final)
            .map_or_else(|| "0".to_owned(), price_text),
        form: text_of(row.ffm_rips.as_deref()),
        concentration: concentration_of(row),
        unit: text_of(row.umm_rips.as_deref()),
    })
}

fn text_of(value: Option<&str>) -> String {
    value.map(str::trim).unwrap_or("").to_owned()
}

fn product_type_of(row: &NtRow) -> String {
    let tipo = text_of(row.tipo_producto.as_deref());
    if !tipo.is_empty() {
        return tipo;
    }
    let clasificacion = text_of(row.clasificacion.as_deref());
    if matches!(normalize_code(&clasificacion).as_str(), "714" | "799") {
        return clasificacion;
    }
    String::new()
}

fn concentration_of(row: &NtRow) -> String {
    for key in CONCENTRATION_KEYS {
        if let Some(value) = row.get(key).map(|v| v.trim().to_owned()) {
            if !value.is_empty() {
                return value;
            }
        }
    }
    text_of(row.upr_rips.as_deref())
}

/// Adds the eligible rows to the catalog in the given order (last wins).
pub fn add_rows(catalog: &mut Catalog, rows: &[NtRow]) {
    for row in rows.iter().filter(|row| is_eligible(row)) {
        if let Some(item) = item_from_row(row) {
            catalog.insert(item);
        }
    }
}

/// Distinct CUMs of the lines (trimmed, non-empty), in a stable order.
pub fn distinct_cums(lines: &[Line]) -> Vec<String> {
    let set: BTreeSet<String> = lines
        .iter()
        .map(|line| line.agreement_cum.trim().to_owned())
        .filter(|cum| !cum.is_empty())
        .collect();
    set.into_iter().collect()
}

/// Builds the catalog of `version` for the given CUMs.
pub fn load_catalog(nt: &dyn NtCatalog, version: &str, cums: Vec<String>) -> Result<Catalog> {
    let fetched = par::map(cums, LOOKUP_WORKERS, |cum| fetch(nt, version, &cum));
    let mut catalog = Catalog::new(version);
    for rows in fetched {
        add_rows(&mut catalog, &rows?);
    }
    Ok(catalog)
}

fn fetch(nt: &dyn NtCatalog, version: &str, cum: &str) -> Result<Vec<NtRow>> {
    let mut rows = nt.lookup(version, "cum_facturar", cum)?;
    let normalized = normalize_code(cum);
    if normalized != cum {
        rows.extend(nt.lookup(version, "cum_facturar", &normalized)?);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nt::MemoryNt;

    fn row(code: &str, cum: &str) -> NtRow {
        NtRow {
            cod_gen_siesa: Some(code.into()),
            cum_facturar: Some(cum.into()),
            tipo_producto: Some("714".into()),
            ..NtRow::default()
        }
    }

    #[test]
    fn a_row_maps_every_field_the_rules_use() {
        let mut r = row("20010", "19950404-1");
        r.precio_contrato = Some("1500.00".parse().expect("dec"));
        r.precio_final = Some("9".parse().expect("dec"));
        r.precio_maximo = Some("99999".parse().expect("dec"));
        r.ffm_rips = Some("TABLETA".into());
        r.upr_rips = Some("500".into());
        r.umm_rips = Some("MG".into());
        let item = item_from_row(&r).expect("item");
        assert_eq!(item.cum, "19950404-1");
        assert_eq!(item.generic_code, "20010");
        assert_eq!(item.product_type, "714");
        assert_eq!(item.price, "1500", "precio_contrato wins, zeros stripped");
        assert_eq!(
            (
                item.form.as_str(),
                item.concentration.as_str(),
                item.unit.as_str()
            ),
            ("TABLETA", "500", "MG")
        );
    }

    #[test]
    fn price_falls_back_to_precio_final_and_then_to_zero() {
        let mut r = row("1", "A");
        r.precio_final = Some("1234.50".parse().expect("dec"));
        assert_eq!(item_from_row(&r).expect("item").price, "1234.5");
        r.precio_final = None;
        r.precio_maximo = Some("5".parse().expect("dec"));
        assert_eq!(
            item_from_row(&r).expect("item").price,
            "0",
            "maximo is not the agreed price"
        );
    }

    #[test]
    fn product_type_uses_tipo_producto_then_clasificacion_then_the_cum_default() {
        let mut r = row("1", "19950404-1");
        r.tipo_producto = Some("799".into());
        assert_eq!(item_from_row(&r).expect("item").product_type, "799");
        r.tipo_producto = None;
        r.clasificacion = Some("799".into());
        assert_eq!(item_from_row(&r).expect("item").product_type, "799");
        r.clasificacion = Some("MEDICAMENTO".into());
        assert_eq!(
            item_from_row(&r).expect("item").product_type,
            "714",
            "a non 714/799 clasificacion is ignored; the CUM has a dash"
        );
        r.cum_facturar = Some("12345".into());
        assert_eq!(item_from_row(&r).expect("item").product_type, "799");
        r.tipo_producto = Some("ABC".into());
        assert_eq!(
            item_from_row(&r).expect("item").product_type,
            "ABC",
            "raw values pass through"
        );
    }

    #[test]
    fn concentration_prefers_an_explicit_key() {
        let mut r = row("1", "A");
        r.upr_rips = Some("UPR".into());
        assert_eq!(item_from_row(&r).expect("item").concentration, "UPR");
        r.extra
            .insert("concentracion".into(), serde_json::json!(250));
        assert_eq!(item_from_row(&r).expect("item").concentration, "250");
    }

    #[test]
    fn rows_without_cum_or_code_are_skipped() {
        assert!(item_from_row(&row("", "A")).is_none());
        assert!(item_from_row(&row("1", " ")).is_none());
        assert!(item_from_row(&NtRow::default()).is_none());
    }

    #[test]
    fn only_validated_or_unmarked_rows_are_eligible() {
        let mut r = row("1", "A");
        assert!(is_eligible(&r));
        r.estado = Some(" validado ".into());
        assert!(is_eligible(&r));
        r.estado = Some("HISTÓRICO".into());
        assert!(!is_eligible(&r));
        r.estado = Some(String::new());
        assert!(is_eligible(&r));
    }

    #[test]
    fn the_last_eligible_duplicate_wins() {
        let mut first = row("9", "55");
        first.precio_final = Some("1".parse().expect("dec"));
        let mut second = row("9", "55");
        second.precio_final = Some("2".parse().expect("dec"));
        let mut stale = row("9", "55");
        stale.precio_final = Some("3".parse().expect("dec"));
        stale.estado = Some("HISTORICO".into());
        let mut catalog = Catalog::new("v");
        add_rows(&mut catalog, &[first, second, stale]);
        assert_eq!(catalog.len(), 1);
        assert_eq!(
            catalog.lookup("9", "55").map(|i| i.price.as_str()),
            Some("2")
        );
    }

    #[test]
    fn the_catalog_is_built_from_the_cums_of_the_lines() {
        let nt = MemoryNt::new().with_version(
            "2026-10",
            vec![
                ("1".into(), row("20010", "19950404-1")),
                ("2".into(), row("30020", "12345")),
                ("3".into(), row("40040", "OTRO")),
            ],
        );
        let lines = vec![
            Line {
                agreement_cum: "19950404-1".into(),
                ..Line::default()
            },
            Line {
                agreement_cum: " 12345 ".into(),
                ..Line::default()
            },
            Line {
                agreement_cum: "19950404-1".into(),
                ..Line::default()
            },
        ];
        let cums = distinct_cums(&lines);
        assert_eq!(cums, vec!["12345", "19950404-1"]);
        let catalog = load_catalog(&nt, "2026-10", cums).expect("catalog");
        assert_eq!(catalog.version, "2026-10");
        assert_eq!(catalog.len(), 2, "OTRO is not in the CSV");
        assert!(catalog.lookup("20010", "19950404-1").is_some());
        assert!(catalog.lookup("30020", "12345").is_some());
    }

    #[test]
    fn a_cum_stored_with_other_case_or_point_zero_is_found_through_its_normalized_form() {
        let nt = MemoryNt::new().with_version("v", vec![("1".into(), row("7", "AB12"))]);
        let catalog = load_catalog(&nt, "v", vec!["ab12.0".into()]).expect("catalog");
        assert!(catalog.lookup("7", "ab12").is_some());
    }

    #[test]
    fn the_version_is_pinned_or_current() {
        let nt = MemoryNt::new()
            .with_version("v1", vec![])
            .with_version("v2", vec![]);
        assert_eq!(resolve_version(&nt, None).expect("current"), "v2");
        assert_eq!(resolve_version(&nt, Some("v1")).expect("pinned"), "v1");
        assert!(resolve_version(&MemoryNt::new(), None).is_err());
    }
}
