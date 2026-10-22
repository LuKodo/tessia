//! Cargue (load) identity helpers (S8).
//!
//! A cargue id is the stem of the convenio CSV (`Convenio_21102026_21102026`,
//! `Manual_<ts>_<hash12>`) made folder-safe, so a re-run on any machine
//! resumes the same cargue. It never contains `__`, which is the separator of
//! bucket keys (`<cargue_id>__<rel path with "/" -> "__">`).

use std::path::Path;
use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;

static UNSAFE_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[^A-Za-z0-9_-]").unwrap_or_else(|_| unreachable!("constant regex"))
});

/// Maximum characters of a cargue id (`cargues.id` is `VARCHAR(190)`).
pub const MAX_ID_CHARS: usize = 190;

/// Provenance of a load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOrigin {
    Manual,
    Medicar,
    Unknown,
}

impl LoadOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            LoadOrigin::Manual => "manual",
            LoadOrigin::Medicar => "medicar",
            LoadOrigin::Unknown => "unknown",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "manual" => Some(LoadOrigin::Manual),
            "medicar" => Some(LoadOrigin::Medicar),
            "unknown" => Some(LoadOrigin::Unknown),
            _ => None,
        }
    }
}

/// Identity of a load derived from its file name (Go `convenio.LoadContext`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadInfo {
    /// Source file name (last path element).
    pub name: String,
    /// Folder-safe id.
    pub id: String,
    /// Range start when the 2nd name part is a `ddMMyyyy` date.
    pub from: Option<NaiveDate>,
    /// Range end when the 3rd name part is a `ddMMyyyy` date.
    pub to: Option<NaiveDate>,
    pub origin: LoadOrigin,
}

/// Derives the load identity from a CSV file name or path.
///
/// The id is the base name without extension and without `_ENPROCESO`, every
/// character outside `[A-Za-z0-9_-]` replaced by `_` and `__` runs collapsed.
/// The range comes from the 2nd and 3rd `_` parts when both are valid dates.
///
/// ```
/// let info = tessia::domain::cargue::load_context("Convenio_21102026_21102026.csv");
/// assert_eq!(info.id, "Convenio_21102026_21102026");
/// assert_eq!(info.from.map(|d| d.to_string()).as_deref(), Some("2026-10-21"));
/// ```
pub fn load_context(file_name: &str) -> LoadInfo {
    let name = Path::new(file_name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name.to_owned());
    let stem = Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.clone());
    let base = stem.replace("_ENPROCESO", "");
    let mut id = UNSAFE_ID.replace_all(base.trim(), "_").into_owned();
    while id.contains("__") {
        id = id.replace("__", "_");
    }
    let parts: Vec<&str> = base.split('_').collect();
    let (from, to) = match (parts.get(1), parts.get(2)) {
        (Some(a), Some(b)) => (name_date(a), name_date(b)),
        _ => (None, None),
    };
    let (from, to) = if from.is_some() && to.is_some() {
        (from, to)
    } else {
        (None, None)
    };
    LoadInfo {
        name,
        id,
        from,
        to,
        origin: LoadOrigin::Unknown,
    }
}

fn name_date(part: &str) -> Option<NaiveDate> {
    if part.len() != 8 || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    NaiveDate::parse_from_str(part, "%d%m%Y").ok()
}

/// True when `id` is usable as a cargue id: 1..=190 characters of
/// `[A-Za-z0-9_-]` and no `__`.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_ID_CHARS
        && !id.contains("__")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_the_folder_safe_stem() {
        let info = load_context("/tmp/Convenio_21102026_21102026_ENPROCESO.csv");
        assert_eq!(info.id, "Convenio_21102026_21102026");
        assert_eq!(info.name, "Convenio_21102026_21102026_ENPROCESO.csv");
        assert_eq!(info.origin, LoadOrigin::Unknown);
    }

    #[test]
    fn unsafe_characters_become_single_underscores() {
        assert_eq!(load_context("Manual  a/b ñ.csv").id, "b_");
        assert_eq!(load_context("Manual a  b.csv").id, "Manual_a_b");
        assert!(is_valid_id(&load_context("Manual (1)__x.csv").id));
    }

    #[test]
    fn range_needs_both_valid_dates() {
        let info = load_context("Convenio_01102026_21102026.csv");
        assert_eq!(
            info.from.map(|d| d.to_string()).as_deref(),
            Some("2026-10-01")
        );
        assert_eq!(
            info.to.map(|d| d.to_string()).as_deref(),
            Some("2026-10-21")
        );
        let bad = load_context("Convenio_01102026_99992026.csv");
        assert_eq!((bad.from, bad.to), (None, None));
    }

    #[test]
    fn valid_ids_exclude_the_key_separator() {
        assert!(is_valid_id("Manual_1_abc"));
        assert!(!is_valid_id("a__b"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("a/b"));
    }
}
