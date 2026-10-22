//! Nota Técnica (NT) catalog: read-only access to the collection kept in Redis.
//!
//! The NT is no longer loaded or copied by this program (S6): a Redis
//! collection that is always available holds every version, already indexed.
//!
//! Observed layout (`<p>` = `redis.nt_prefix`, default `nt:`):
//!
//! | key | type | content |
//! |---|---|---|
//! | `<p>actual` | string | current version |
//! | `<p>versiones` | zset | known versions |
//! | `<p><ver>:meta` | hash | version metadata |
//! | `<p><ver>:filas` | hash | field `<id>` or `<id>#N` -> JSON row |
//! | `<p><ver>:idx:<campo>:<valor>` | set | ids of the rows with that value |
//!
//! Indexed fields: `cod_auf`, `cod_mutual`, `cum_facturar`, `contrato`, `estado`.
//!
//! Phases use the [`NtCatalog`] trait through `AppContext::nt`; tests use
//! [`MemoryNt`], production [`RedisNt`].
//!
//! ```
//! use tessia::nt::{MemoryNt, NtCatalog, NtRow};
//!
//! let mut row = NtRow::default();
//! row.cum_facturar = Some("19900001-1".into());
//! row.estado = Some("VIGENTE".into());
//! let nt = MemoryNt::new().with_version("2026-10", vec![("77".into(), row)]);
//! assert_eq!(nt.current_version().unwrap().as_deref(), Some("2026-10"));
//! let rows = nt.lookup("2026-10", "cum_facturar", "19900001-1").unwrap();
//! assert_eq!(rows[0].id, "77");
//! ```

mod memory;
mod redis_nt;
mod row;

use std::collections::BTreeMap;

pub use memory::MemoryNt;
pub use redis_nt::RedisNt;
pub use row::NtRow;

/// Fields indexed by the collection (`idx:<campo>:<valor>`).
pub const INDEXED_FIELDS: [&str; 5] = [
    "cod_auf",
    "cod_mutual",
    "cum_facturar",
    "contrato",
    "estado",
];

#[derive(Debug, thiserror::Error)]
pub enum NtError {
    #[error("Nota Técnica (Redis): {0}")]
    Redis(String),
    #[error("Nota Técnica: la fila {id} de la versión {version} no es JSON válido: {message}")]
    BadRow {
        version: String,
        id: String,
        message: String,
    },
}

impl From<redis::RedisError> for NtError {
    fn from(err: redis::RedisError) -> Self {
        NtError::Redis(err.to_string())
    }
}

pub type NtResult<T> = Result<T, NtError>;

/// Read-only view of the Nota Técnica collection.
pub trait NtCatalog: Send + Sync {
    /// The current version (`<p>actual`), or `None` when the collection is empty.
    fn current_version(&self) -> NtResult<Option<String>>;

    /// All known versions, oldest first.
    fn versions(&self) -> NtResult<Vec<String>>;

    /// The version's metadata hash (empty when unknown).
    fn meta(&self, version: &str) -> NtResult<BTreeMap<String, String>>;

    /// Rows whose `field` equals `value` (resolved through the `idx` sets),
    /// ordered by id. Every `<id>#N` variant of a matching id is included.
    fn lookup(&self, version: &str, field: &str, value: &str) -> NtResult<Vec<NtRow>>;

    /// The row stored under hash field `id` (`<id>` or `<id>#N`).
    fn row(&self, version: &str, id: &str) -> NtResult<Option<NtRow>>;
}
