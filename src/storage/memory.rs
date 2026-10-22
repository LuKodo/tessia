//! In-memory [`ObjectStore`] for tests.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use super::{ListPage, ObjectInfo, ObjectStore, StorageError, StorageResult};
use crate::util::fsx;

/// Thread-safe in-memory bucket. Listing is ordered by name; the cursor is the
/// name of the first object of the next page.
#[derive(Debug, Default)]
pub struct MemoryStore {
    objects: Mutex<BTreeMap<String, (Vec<u8>, String)>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, (Vec<u8>, String)>> {
        self.objects
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Number of stored objects.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Names of all objects, ordered.
    pub fn keys(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }
}

impl ObjectStore for MemoryStore {
    fn put_bytes(&self, key: &str, data: &[u8], content_type: &str) -> StorageResult<()> {
        self.lock()
            .insert(key.to_owned(), (data.to_vec(), content_type.to_owned()));
        Ok(())
    }

    fn put_file(&self, key: &str, path: &Path, content_type: &str) -> StorageResult<()> {
        let data = std::fs::read(path)?;
        self.put_bytes(key, &data, content_type)
    }

    fn get_bytes(&self, key: &str) -> StorageResult<Vec<u8>> {
        self.lock()
            .get(key)
            .map(|(data, _)| data.clone())
            .ok_or_else(|| StorageError::NotFound(key.to_owned()))
    }

    fn get_to_file(&self, key: &str, dest: &Path) -> StorageResult<u64> {
        let data = self.get_bytes(key)?;
        fsx::atomic_write(dest, &data).map_err(|err| StorageError::Failed(format!("{err:#}")))?;
        Ok(data.len() as u64)
    }

    fn exists(&self, key: &str) -> StorageResult<bool> {
        Ok(self.lock().contains_key(key))
    }

    fn delete(&self, key: &str) -> StorageResult<bool> {
        Ok(self.lock().remove(key).is_some())
    }

    fn list(&self, start: Option<&str>, limit: usize) -> StorageResult<ListPage> {
        let objects = self.lock();
        let mut rest = objects
            .iter()
            .filter(|(name, _)| start.is_none_or(|s| name.as_str() >= s));
        let items: Vec<ObjectInfo> = rest
            .by_ref()
            .take(limit.max(1))
            .map(|(name, (data, content_type))| ObjectInfo {
                name: name.clone(),
                size: data.len() as u64,
                content_type: content_type.clone(),
                updated_at: None,
            })
            .collect();
        let next_start = rest.next().map(|(name, _)| name.clone());
        Ok(ListPage { items, next_start })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_overwrite_exists_delete() {
        let store = MemoryStore::new();
        assert!(!store.exists("a").expect("exists"));
        store.put_bytes("a", b"1", "text/plain").expect("put");
        store
            .put_bytes("a", b"22", "text/plain")
            .expect("overwrite");
        assert_eq!(store.get_bytes("a").expect("get"), b"22");
        assert!(store.exists("a").expect("exists"));
        assert!(matches!(
            store.get_bytes("nope"),
            Err(StorageError::NotFound(_))
        ));
        assert!(store.delete("a").expect("delete"));
        assert!(!store.delete("a").expect("delete again"));
    }

    #[test]
    fn listing_pages_through_a_cursor() {
        let store = MemoryStore::new();
        for name in ["c", "a", "e", "b", "d"] {
            store
                .put_bytes(name, name.as_bytes(), "text/plain")
                .expect("put");
        }
        let first = store.list(None, 2).expect("page");
        assert_eq!(
            first
                .items
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        let second = store.list(first.next_start.as_deref(), 2).expect("page");
        assert_eq!(
            second
                .items
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>(),
            ["c", "d"]
        );
        let last = store.list(second.next_start.as_deref(), 2).expect("page");
        assert_eq!((last.items.len(), last.next_start), (1, None));
    }

    #[test]
    fn list_prefix_follows_all_pages() {
        let store = MemoryStore::new();
        for i in 0..450 {
            store
                .put_bytes(&format!("C1__{i:04}"), b"x", "text/plain")
                .expect("put");
        }
        store.put_bytes("C2__x", b"x", "text/plain").expect("other");
        assert_eq!(store.list_prefix("C1__").expect("list").len(), 450);
    }

    #[test]
    fn files_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("in.bin");
        std::fs::write(&src, b"payload").expect("write");
        let store = MemoryStore::new();
        store
            .put_file("k", &src, "application/octet-stream")
            .expect("put");
        let out = dir.path().join("sub/out.bin");
        assert_eq!(store.get_to_file("k", &out).expect("get"), 7);
        assert_eq!(std::fs::read(out).expect("read"), b"payload");
    }
}
