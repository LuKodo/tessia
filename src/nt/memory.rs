//! In-memory [`NtCatalog`] for tests and offline use.

use std::collections::BTreeMap;

use super::row::base_id;
use super::{NtCatalog, NtResult, NtRow};

#[derive(Debug, Clone, Default)]
struct Version {
    meta: BTreeMap<String, String>,
    rows: BTreeMap<String, NtRow>,
}

/// An NT held in memory. Build it with the `with_*` methods.
#[derive(Debug, Clone, Default)]
pub struct MemoryNt {
    current: Option<String>,
    versions: BTreeMap<String, Version>,
}

impl MemoryNt {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a version with `(hash field, row)` pairs and makes it current.
    #[must_use]
    pub fn with_version(mut self, version: &str, rows: Vec<(String, NtRow)>) -> Self {
        let rows = rows
            .into_iter()
            .map(|(id, mut row)| {
                row.id.clone_from(&id);
                (id, row)
            })
            .collect();
        self.versions.insert(
            version.to_owned(),
            Version {
                meta: BTreeMap::new(),
                rows,
            },
        );
        self.current = Some(version.to_owned());
        self
    }

    /// Sets the metadata hash of a version.
    #[must_use]
    pub fn with_meta(mut self, version: &str, meta: &[(&str, &str)]) -> Self {
        let entry = self.versions.entry(version.to_owned()).or_default();
        entry.meta = meta
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        self
    }

    /// Marks `version` as the current one.
    #[must_use]
    pub fn with_current(mut self, version: &str) -> Self {
        self.current = Some(version.to_owned());
        self
    }
}

impl NtCatalog for MemoryNt {
    fn current_version(&self) -> NtResult<Option<String>> {
        Ok(self.current.clone())
    }

    fn versions(&self) -> NtResult<Vec<String>> {
        Ok(self.versions.keys().cloned().collect())
    }

    fn meta(&self, version: &str) -> NtResult<BTreeMap<String, String>> {
        Ok(self
            .versions
            .get(version)
            .map(|v| v.meta.clone())
            .unwrap_or_default())
    }

    fn lookup(&self, version: &str, field: &str, value: &str) -> NtResult<Vec<NtRow>> {
        let Some(version) = self.versions.get(version) else {
            return Ok(Vec::new());
        };
        // A match on any row of an id brings every `<id>#N` variant, like the
        // Redis collection, whose sets list ids.
        let matching: std::collections::BTreeSet<&str> = version
            .rows
            .values()
            .filter(|row| row.get(field).as_deref() == Some(value))
            .map(|row| base_id(&row.id))
            .collect();
        Ok(version
            .rows
            .values()
            .filter(|row| matching.contains(base_id(&row.id)))
            .cloned()
            .collect())
    }

    fn row(&self, version: &str, id: &str) -> NtResult<Option<NtRow>> {
        Ok(self
            .versions
            .get(version)
            .and_then(|v| v.rows.get(id))
            .cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cum: &str, estado: &str) -> NtRow {
        NtRow {
            cum_facturar: Some(cum.into()),
            estado: Some(estado.into()),
            ..NtRow::default()
        }
    }

    fn sample() -> MemoryNt {
        MemoryNt::new()
            .with_version("v1", vec![("1".into(), row("A", "VIGENTE"))])
            .with_version(
                "v2",
                vec![
                    ("1".into(), row("A", "VIGENTE")),
                    ("1#1".into(), row("A", "HISTORICO")),
                    ("2".into(), row("B", "VIGENTE")),
                ],
            )
            .with_meta("v2", &[("filas", "3")])
    }

    #[test]
    fn current_is_the_last_added_unless_changed() {
        let nt = sample();
        assert_eq!(
            nt.current_version().expect("current").as_deref(),
            Some("v2")
        );
        assert_eq!(
            nt.clone()
                .with_current("v1")
                .current_version()
                .expect("current")
                .as_deref(),
            Some("v1")
        );
        assert_eq!(nt.versions().expect("versions"), ["v1", "v2"]);
        assert_eq!(MemoryNt::new().current_version().expect("empty"), None);
    }

    #[test]
    fn lookup_returns_all_variants_of_a_matching_id_ordered() {
        let nt = sample();
        let rows = nt.lookup("v2", "cum_facturar", "A").expect("lookup");
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["1", "1#1"]
        );
        let only_historic = nt.lookup("v2", "estado", "HISTORICO").expect("lookup");
        assert_eq!(
            only_historic.len(),
            2,
            "the id of a matching variant brings its siblings"
        );
        assert!(
            nt.lookup("v2", "cum_facturar", "ZZ")
                .expect("none")
                .is_empty()
        );
        assert!(
            nt.lookup("nope", "cum_facturar", "A")
                .expect("unknown version")
                .is_empty()
        );
    }

    #[test]
    fn row_and_meta_are_exact() {
        let nt = sample();
        assert_eq!(
            nt.row("v2", "1#1")
                .expect("row")
                .and_then(|r| r.estado)
                .as_deref(),
            Some("HISTORICO")
        );
        assert!(nt.row("v2", "9").expect("row").is_none());
        assert_eq!(
            nt.meta("v2")
                .expect("meta")
                .get("filas")
                .map(String::as_str),
            Some("3")
        );
        assert!(nt.meta("v1").expect("meta").is_empty());
    }
}
