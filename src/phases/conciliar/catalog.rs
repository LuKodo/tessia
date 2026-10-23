//! The nota técnica catalog `conciliar` matches XML identifiers against.
//!
//! Port of Go `convenio.Catalog` (items keyed by `generic code|CUM`,
//! `MatchIdentifier`). Go filled it from the xlsx workbook; here it is built
//! from the rows of the NT version pinned at preparation time
//! ([`Catalog::from_nt_rows`]).
//!
//! # NtRow to catalog item (this phase's own adapter)
//!
//! | catalog `Item` | `NtRow` field |
//! |---|---|
//! | `cum` (CUM to bill) | `cum_facturar` |
//! | `generic_code` (Siesa generic code) | `cod_gen_siesa` |
//! | `product_type` (714/799) | `tipo_producto` (`clasificacion` when empty), normalized like Go |
//! | `price` | `precio_contrato`, else `precio_final`, as `0.##`; `"0"` when absent |
//! | `form` | `ffm_rips` |
//! | `concentration` | `umm_rips` |
//! | `unit` | `upr_rips` |
//! | `plu` | extra key `plu` or `referencia` (`""` when absent) |
//!
//! The Go workbook columns were forma (E), concentración (F) and unidad (G); the
//! three `*_rips` fields are mapped in that positional order. This is an
//! assumption to confirm against the real NT (open question for the parent).
//! Rows without CUM, product type or generic code are skipped, as in Go.

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::nt::NtRow;
use crate::util::decimal::round2;

/// One nota técnica entry (Go `convenio.Item`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    /// CUM to bill.
    pub cum: String,
    /// 714 medicine, 799 other service (or the raw value).
    pub product_type: String,
    /// Siesa generic code.
    pub generic_code: String,
    /// Agreed price as `0.##` (`"0"` when absent or invalid).
    pub price: String,
    pub description: String,
    pub form: String,
    pub concentration: String,
    pub unit: String,
    /// Siesa PLU, empty when the NT has none.
    pub plu: String,
}

/// Items indexed by generic code and CUM.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    items: BTreeMap<String, Item>,
}

/// Trims, upper-cases and drops a trailing `.0` (numeric cells).
pub fn normalize_code(s: &str) -> String {
    let v = s.trim().to_uppercase();
    v.strip_suffix(".0").map(str::to_owned).unwrap_or(v)
}

fn key(code: &str, cum: &str) -> String {
    format!("{}|{}", normalize_code(code), normalize_code(cum))
}

/// The Siesa generic code: the PLU up to the first `_`.
pub fn generic_code_from_plu(plu: &str) -> String {
    let first = plu.trim().split('_').next().unwrap_or("");
    normalize_code(first)
}

/// Keeps 714/799; an empty or formula value is derived from the CUM (a dash or
/// more than 14 characters means a medicine).
pub fn normalize_product_type(typ: &str, cum: &str) -> String {
    let t = normalize_code(typ);
    if t == "714" || t == "799" {
        return t;
    }
    if t.is_empty() || t.starts_with('=') {
        let c = normalize_code(cum);
        return if c.contains('-') || c.chars().count() > 14 {
            "714".to_owned()
        } else {
            "799".to_owned()
        };
    }
    t
}

/// `0.##`: at most two decimals, no trailing zeros (`"235"`, `"333.5"`).
pub fn format_price(price: Decimal) -> String {
    round2(price).normalize().to_string()
}

impl Catalog {
    /// Builds a catalog from already normalized items; a repeated key keeps the
    /// last item.
    pub fn new(items: Vec<Item>) -> Self {
        let mut c = Catalog::default();
        for it in items {
            c.items.insert(key(&it.generic_code, &it.cum), it);
        }
        c
    }

    /// Builds the catalog from NT rows (see the module docs for the mapping).
    pub fn from_nt_rows<'a>(rows: impl IntoIterator<Item = &'a NtRow>) -> Self {
        Self::new(rows.into_iter().filter_map(item_from_row).collect())
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Splits a FEV XML item identifier `<PLU>-<CUM>` against the catalog: the
    /// CUM is a catalog CUM that ends the identifier and the entry must share
    /// either the Siesa generic code (the PLU up to the first `_`) or the PLU.
    /// Returns the XML PLU and CUM and the entry.
    pub fn match_identifier(&self, id: &str) -> Option<(String, String, &Item)> {
        let v = id.trim();
        for (i, _) in v.match_indices('-') {
            let (p, q) = (&v[..i], &v[i + 1..]);
            if p.is_empty() || q.is_empty() {
                continue;
            }
            if let Some(it) = self.items.get(&key(&generic_code_from_plu(p), q)) {
                return Some((p.to_owned(), q.to_owned(), it));
            }
            let (np, nq) = (normalize_code(p), normalize_code(q));
            let by_plu = self.items.values().find(|it| {
                !it.plu.is_empty() && normalize_code(&it.plu) == np && normalize_code(&it.cum) == nq
            });
            if let Some(it) = by_plu {
                return Some((p.to_owned(), q.to_owned(), it));
            }
        }
        None
    }
}

fn text(v: Option<&String>) -> String {
    v.map(|s| s.trim().to_owned()).unwrap_or_default()
}

fn extra_text(row: &NtRow, names: &[&str]) -> String {
    names
        .iter()
        .find_map(|n| row.extra.get(*n).and_then(|v| v.as_str()))
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

fn item_from_row(row: &NtRow) -> Option<Item> {
    let cum = text(row.cum_facturar.as_ref());
    let typ_raw = row
        .tipo_producto
        .as_ref()
        .filter(|t| !t.trim().is_empty())
        .or(row.clasificacion.as_ref());
    let product_type = normalize_product_type(&text(typ_raw), &cum);
    let code = text(row.cod_gen_siesa.as_ref());
    if cum.is_empty() || product_type.is_empty() || code.is_empty() {
        return None;
    }
    let price = row
        .precio_contrato
        .or(row.precio_final)
        .map(format_price)
        .unwrap_or_else(|| "0".to_owned());
    Some(Item {
        cum,
        product_type,
        generic_code: code,
        price,
        description: extra_text(row, &["descripcion", "nombre"]),
        form: text(row.ffm_rips.as_ref()),
        concentration: text(row.umm_rips.as_ref()),
        unit: text(row.upr_rips.as_ref()),
        plu: extra_text(row, &["plu", "referencia"]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(cum: &str, code: &str, plu: &str, price: &str) -> Item {
        Item {
            cum: cum.into(),
            product_type: "714".into(),
            generic_code: code.into(),
            price: price.into(),
            plu: plu.into(),
            ..Item::default()
        }
    }

    #[test]
    fn matches_by_generic_code_by_plu_and_rejects_the_rest() {
        let cat = Catalog::new(vec![
            item("27363-1", "23", "23_11_19", "235"),
            item("20021574-2", "57", "", "333.5"),
            item("12345", "300", "", "400"),
        ]);
        let (plu, cum, it) = cat.match_identifier("23_11_19-27363-1").expect("by code");
        assert_eq!(
            (plu.as_str(), cum.as_str(), it.price.as_str()),
            ("23_11_19", "27363-1", "235")
        );
        // The dash inside the CUM is tried after the first split fails.
        assert!(cat.match_identifier("57_20_7-20021574-2").is_some());
        // Same CUM, other generic code and PLU: no match.
        assert!(cat.match_identifier("99_1_1-27363-1").is_none());
        // Exact PLU match even when the generic code differs.
        let by_plu = Catalog::new(vec![item("5555", "ZZ", "AB_9", "1")]);
        assert!(by_plu.match_identifier("AB_9-5555").is_some());
        assert!(by_plu.match_identifier("AB_8-5555").is_none());
        assert!(cat.match_identifier("sin-guion-valido-").is_none());
        // An empty PLU part is skipped; the next dash splits "27363" / "1".
        assert!(cat.match_identifier("-27363-1").is_none());
    }

    #[test]
    fn product_type_is_derived_from_the_cum_when_missing() {
        assert_eq!(normalize_product_type("714.0", "x"), "714");
        assert_eq!(normalize_product_type("", "123-4"), "714");
        assert_eq!(normalize_product_type("=A1", "12345"), "799");
        assert_eq!(normalize_product_type("", "123456789012345"), "714");
        assert_eq!(normalize_product_type("abc", "1"), "ABC");
    }

    #[test]
    fn nt_rows_become_items() {
        let row = NtRow {
            cum_facturar: Some("27363-1".into()),
            cod_gen_siesa: Some("23".into()),
            tipo_producto: Some("714".into()),
            precio_contrato: Some("235.00".parse().expect("decimal")),
            ffm_rips: Some("COLFF001".into()),
            umm_rips: Some("11".into()),
            upr_rips: Some("1".into()),
            ..NtRow::default()
        };
        let incomplete = NtRow {
            cum_facturar: Some("1".into()),
            ..NtRow::default()
        };
        let cat = Catalog::from_nt_rows([&row, &incomplete]);
        assert_eq!(cat.len(), 1);
        let (_, _, it) = cat.match_identifier("23_1_1-27363-1").expect("match");
        assert_eq!(
            (
                it.price.as_str(),
                it.form.as_str(),
                it.concentration.as_str(),
                it.unit.as_str()
            ),
            ("235", "COLFF001", "11", "1")
        );
    }

    #[test]
    fn missing_price_is_zero_and_fallback_is_precio_final() {
        let mut row = NtRow {
            cum_facturar: Some("9-1".into()),
            cod_gen_siesa: Some("9".into()),
            ..NtRow::default()
        };
        assert_eq!(item_from_row(&row).expect("item").price, "0");
        row.precio_final = Some("10.5".parse().expect("decimal"));
        assert_eq!(item_from_row(&row).expect("item").price, "10.5");
    }
}
