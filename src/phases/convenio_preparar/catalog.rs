//! The catalog the rules cross against (port of the lookup half of Go
//! `convenio/catalog.go`).
//!
//! Go read the catalog from a nota técnica workbook. Here the NT comes from
//! Redis (S6): [`super::nt_catalog`] builds a [`Catalog`] from `NtRow`s, and the
//! rule engine only sees this module's types.

use std::collections::HashMap;

/// One nota técnica entry as the rules use it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    /// CUM to bill.
    pub cum: String,
    /// 714 medicine, 799 other service (or the raw value).
    pub product_type: String,
    /// Siesa generic code.
    pub generic_code: String,
    /// Agreed price, `0.##` (`0` when absent).
    pub price: String,
    /// Pharmaceutical form (RIPS).
    pub form: String,
    /// Concentration (RIPS).
    pub concentration: String,
    /// Measure unit (RIPS).
    pub unit: String,
}

/// Entries indexed by `normCode(generic code)|normCode(CUM)`.
///
/// A repeated key keeps the last inserted item (Go `LoadCatalog`).
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    /// NT version the entries were read from (pinned in the preparation).
    pub version: String,
    items: HashMap<String, Item>,
}

impl Catalog {
    pub fn new(version: &str) -> Self {
        Self {
            version: version.to_owned(),
            items: HashMap::new(),
        }
    }

    /// Adds (or replaces) the entry of its key.
    pub fn insert(&mut self, item: Item) {
        self.items
            .insert(catalog_key(&item.generic_code, &item.cum), item);
    }

    /// Entry for a generic code and the convenio CUM.
    pub fn lookup(&self, generic_code: &str, cum: &str) -> Option<&Item> {
        self.items.get(&catalog_key(generic_code, cum))
    }

    /// Number of distinct entries.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Trims, upper-cases and drops a trailing `.0` (numeric cells).
///
/// ```
/// use tessia::phases::convenio_preparar::catalog::normalize_code;
/// assert_eq!(normalize_code(" ab12.0 "), "AB12");
/// ```
pub fn normalize_code(text: &str) -> String {
    let upper = text.trim().to_uppercase();
    upper
        .strip_suffix(".0")
        .map_or(upper.clone(), str::to_owned)
}

/// `normCode(code)|normCode(cum)`.
pub fn catalog_key(code: &str, cum: &str) -> String {
    format!("{}|{}", normalize_code(code), normalize_code(cum))
}

/// The Siesa generic code: the PLU up to the first `_`.
///
/// ```
/// use tessia::phases::convenio_preparar::catalog::generic_code_from_plu;
/// assert_eq!(generic_code_from_plu(" 20010_01_123 "), "20010");
/// ```
pub fn generic_code_from_plu(plu: &str) -> String {
    let trimmed = plu.trim();
    normalize_code(trimmed.split('_').next().unwrap_or(trimmed))
}

/// Keeps 714/799; an empty or formula value is derived from the CUM (a dash or
/// more than 14 characters means a medicine).
pub fn normalize_product_type(product_type: &str, cum: &str) -> String {
    let normalized = normalize_code(product_type);
    if normalized == "714" || normalized == "799" {
        return normalized;
    }
    if normalized.is_empty() || normalized.starts_with('=') {
        let cum = normalize_code(cum);
        return if cum.contains('-') || cum.len() > 14 {
            "714".to_owned()
        } else {
            "799".to_owned()
        };
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_trimmed_upper_cased_and_lose_a_trailing_point_zero() {
        assert_eq!(normalize_code(" 123.0 "), "123");
        assert_eq!(normalize_code("ab-1"), "AB-1");
        assert_eq!(normalize_code("1.05"), "1.05");
        assert_eq!(catalog_key("20010.0", " 1995-1 "), "20010|1995-1");
    }

    #[test]
    fn the_generic_code_is_the_plu_up_to_the_first_underscore() {
        assert_eq!(generic_code_from_plu("20010_01"), "20010");
        assert_eq!(generic_code_from_plu("abc"), "ABC");
        assert_eq!(generic_code_from_plu(""), "");
    }

    #[test]
    fn product_type_defaults_follow_the_cum() {
        assert_eq!(normalize_product_type("714", "x"), "714");
        assert_eq!(normalize_product_type(" 799 ", "19950404-1"), "799");
        assert_eq!(normalize_product_type("", "19950404-1"), "714", "dash");
        assert_eq!(normalize_product_type("", "123456789012345"), "714", "long");
        assert_eq!(normalize_product_type("=A1", "12345"), "799");
        assert_eq!(normalize_product_type("abc", "12345"), "ABC", "raw passes");
    }

    #[test]
    fn a_repeated_key_keeps_the_last_item() {
        let mut catalog = Catalog::new("v1");
        for price in ["1", "2"] {
            catalog.insert(Item {
                cum: "55".into(),
                generic_code: "9".into(),
                price: price.into(),
                ..Item::default()
            });
        }
        assert_eq!(catalog.len(), 1);
        assert_eq!(
            catalog.lookup("9", "55").map(|i| i.price.as_str()),
            Some("2")
        );
        assert!(catalog.lookup("9", "56").is_none());
    }
}
