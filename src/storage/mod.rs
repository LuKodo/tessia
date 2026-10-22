//! Object storage for cargue artifacts (the bucket of S5) and the per-machine
//! temp workdir.
//!
//! ```text
//!  phase ──► Workdir (local temp, original folder layout, deleted on Drop)
//!    │           ▲ fetch                        │ publish
//!    ▼           │                              ▼
//!  Artifacts ──► ObjectStore (CassandraStore | MemoryStore)   +   artefactos index (Postgres)
//! ```
//!
//! * [`ObjectStore`]: the small trait over the bucket (`put/get/exists/delete/list`);
//! * [`CassandraStore`]: the real implementation over the Cassandra storage API;
//! * [`MemoryStore`]: an in-memory fake for tests;
//! * [`Artifacts`]: publish/fetch by `(cargue_id, rel_path)`, verifying SHA-256 and
//!   keeping the `artefactos` index;
//! * [`Workdir`]: the temp folder a phase works in.
//!
//! The bucket is flat. The object key of `(cargue_id, rel_path)` is
//! `<cargue_id>__<rel_path with "/" replaced by "__">` (see [`object_key`]) and is
//! never parsed back: the `artefactos` table is the index.

mod artifacts;
mod cassandra;
mod memory;
mod workdir;

use std::path::Path;

pub use artifacts::{Artifacts, content_type_for, object_key, validate_rel_path};
pub use cassandra::CassandraStore;
pub use memory::MemoryStore;
pub use workdir::{Workdir, layout};

/// Failure of an object store operation.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// The object does not exist.
    #[error("objeto no encontrado: {0}")]
    NotFound(String),
    /// The service answered with an error or could not be reached.
    #[error("almacenamiento de objetos: {0}")]
    Failed(String),
    /// A local file operation failed.
    #[error("archivo local: {0}")]
    Io(#[from] std::io::Error),
}

pub type StorageResult<T> = Result<T, StorageError>;

/// Metadata of a stored object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    pub name: String,
    pub size: u64,
    pub content_type: String,
    /// As the service reports it (RFC 3339), when it does.
    pub updated_at: Option<String>,
}

/// One page of a listing. `next_start` is an opaque cursor: pass it back as
/// `start` to continue; `None` means the end.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListPage {
    pub items: Vec<ObjectInfo>,
    pub next_start: Option<String>,
}

/// The bucket. Same-name `put` overwrites; names are flat.
pub trait ObjectStore: Send + Sync {
    /// Stores `data` under `key`.
    fn put_bytes(&self, key: &str, data: &[u8], content_type: &str) -> StorageResult<()>;

    /// Stores the file at `path` under `key`.
    fn put_file(&self, key: &str, path: &Path, content_type: &str) -> StorageResult<()>;

    /// The object's bytes; [`StorageError::NotFound`] when it does not exist.
    fn get_bytes(&self, key: &str) -> StorageResult<Vec<u8>>;

    /// Downloads the object to `dest` (written atomically, mode 0600) and
    /// returns its size.
    fn get_to_file(&self, key: &str, dest: &Path) -> StorageResult<u64>;

    /// Whether the object exists.
    fn exists(&self, key: &str) -> StorageResult<bool>;

    /// Deletes the object; false when it did not exist.
    fn delete(&self, key: &str) -> StorageResult<bool>;

    /// One page of at most `limit` objects starting at the opaque cursor
    /// `start` (`None` = the beginning).
    fn list(&self, start: Option<&str>, limit: usize) -> StorageResult<ListPage>;

    /// Every object whose name starts with `prefix`, following the cursor.
    fn list_prefix(&self, prefix: &str) -> StorageResult<Vec<ObjectInfo>> {
        let mut out = Vec::new();
        let mut start: Option<String> = None;
        loop {
            let page = self.list(start.as_deref(), 200)?;
            out.extend(
                page.items
                    .into_iter()
                    .filter(|o| o.name.starts_with(prefix)),
            );
            match page.next_start {
                Some(next) if !next.is_empty() => start = Some(next),
                _ => return Ok(out),
            }
        }
    }
}
