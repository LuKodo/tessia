//! Publish/fetch of cargue artifacts: bucket object + `artefactos` index row.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use super::{ObjectStore, StorageError, Workdir};
use crate::domain::cargue::is_valid_id;
use crate::stores::artefactos::{Artefacto, ArtefactosStore};

/// Checks a relative path: non-empty, `/`-separated, no leading `/`, no `.`/`..`
/// segments, no `\`, and no `__` (the key separator).
pub fn validate_rel_path(rel: &str) -> Result<()> {
    if rel.is_empty() || rel.starts_with('/') || rel.contains('\\') || rel.contains("__") {
        bail!("ruta relativa de artefacto inválida: {rel:?}");
    }
    if rel
        .split('/')
        .any(|s| s.is_empty() || s == "." || s == "..")
    {
        bail!("ruta relativa de artefacto inválida: {rel:?}");
    }
    Ok(())
}

/// Bucket object name of `(cargue_id, rel_path)`: `<cargue_id>__<rel with "/"
/// replaced by "__">`. Fails for invalid ids or paths.
///
/// ```
/// let key = tessia::storage::object_key("Convenio_21102026_21102026", "XML/FEV1_XML_PDF.zip").unwrap();
/// assert_eq!(key, "Convenio_21102026_21102026__XML__FEV1_XML_PDF.zip");
/// ```
pub fn object_key(cargue_id: &str, rel_path: &str) -> Result<String> {
    if !is_valid_id(cargue_id) {
        bail!("id de cargue inválido: {cargue_id:?}");
    }
    validate_rel_path(rel_path)?;
    Ok(format!("{cargue_id}__{}", rel_path.replace('/', "__")))
}

/// Content type by extension (the bucket stores it; downloads do not depend
/// on it).
pub fn content_type_for(rel_path: &str) -> &'static str {
    match rel_path
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("xml") => "application/xml",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        Some("json") => "application/json",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn sha256_file(path: &Path) -> Result<(String, i64)> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("no se pudo abrir {}", path.display()))?;
    let mut hasher = Sha256::new();
    let size = std::io::copy(&mut file, &mut hasher)?;
    Ok((hex::encode(hasher.finalize()), size as i64))
}

/// Publishes and fetches artifacts of cargues.
///
/// ```no_run
/// # use tessia::context::AppContext;
/// # fn demo(ctx: &AppContext) -> anyhow::Result<()> {
/// let artifacts = ctx.artifacts();
/// artifacts.publish_bytes("Convenio_21102026_21102026", "derechos/FEV1.json", b"{}")?;
/// let bytes = artifacts.fetch_bytes("Convenio_21102026_21102026", "derechos/FEV1.json")?;
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct Artifacts {
    store: Arc<dyn ObjectStore>,
    index: ArtefactosStore,
}

impl Artifacts {
    pub fn new(store: Arc<dyn ObjectStore>, index: ArtefactosStore) -> Self {
        Self { store, index }
    }

    /// Uploads `data` as `(cargue_id, rel_path)` and records it in the index
    /// (replacing a previous version).
    pub fn publish_bytes(&self, cargue_id: &str, rel_path: &str, data: &[u8]) -> Result<Artefacto> {
        let key = object_key(cargue_id, rel_path)?;
        let content_type = content_type_for(rel_path);
        self.store
            .put_bytes(&key, data, content_type)
            .with_context(|| format!("no se pudo publicar {rel_path}"))?;
        self.record(
            cargue_id,
            rel_path,
            &key,
            data.len() as i64,
            &sha256_hex(data),
            content_type,
        )
    }

    /// Uploads the local file as `(cargue_id, rel_path)`.
    pub fn publish_file(&self, cargue_id: &str, rel_path: &str, local: &Path) -> Result<Artefacto> {
        let key = object_key(cargue_id, rel_path)?;
        let content_type = content_type_for(rel_path);
        let (sha, size) = sha256_file(local)?;
        self.store
            .put_file(&key, local, content_type)
            .with_context(|| format!("no se pudo publicar {rel_path}"))?;
        self.record(cargue_id, rel_path, &key, size, &sha, content_type)
    }

    /// Publishes every file of the workdir (all of it, or only the files under
    /// `dir`) and returns the index rows.
    pub fn publish_workdir(&self, workdir: &Workdir, dir: Option<&str>) -> Result<Vec<Artefacto>> {
        let files = match dir {
            Some(dir) => workdir.files_in(dir)?,
            None => workdir.files()?,
        };
        files
            .iter()
            .map(|rel| self.publish_file(workdir.cargue_id(), rel, &workdir.file(rel)?))
            .collect()
    }

    fn record(
        &self,
        cargue_id: &str,
        rel_path: &str,
        key: &str,
        size: i64,
        sha: &str,
        content_type: &str,
    ) -> Result<Artefacto> {
        self.index
            .upsert(cargue_id, rel_path, key, size, sha, content_type)?;
        self.index
            .get(cargue_id, rel_path)?
            .context("el índice de artefactos no devolvió la fila recién escrita")
    }

    fn indexed(&self, cargue_id: &str, rel_path: &str) -> Result<Artefacto> {
        validate_rel_path(rel_path)?;
        self.index.get(cargue_id, rel_path)?.with_context(|| {
            format!("el artefacto {rel_path} del cargue {cargue_id} no está publicado")
        })
    }

    /// The artifact's bytes, with their SHA-256 and size checked against the
    /// index.
    pub fn fetch_bytes(&self, cargue_id: &str, rel_path: &str) -> Result<Vec<u8>> {
        let row = self.indexed(cargue_id, rel_path)?;
        let data = self
            .store
            .get_bytes(&row.key)
            .map_err(|err| missing_or(err, rel_path))?;
        verify(&row, &sha256_hex(&data), data.len() as i64)?;
        Ok(data)
    }

    /// Downloads the artifact to `dest`, verifying its SHA-256 and size; a
    /// corrupted download is deleted.
    pub fn fetch_to_file(&self, cargue_id: &str, rel_path: &str, dest: &Path) -> Result<()> {
        let row = self.indexed(cargue_id, rel_path)?;
        self.store
            .get_to_file(&row.key, dest)
            .map_err(|err| missing_or(err, rel_path))?;
        let (sha, size) = sha256_file(dest)?;
        if let Err(err) = verify(&row, &sha, size) {
            let _ = std::fs::remove_file(dest);
            return Err(err);
        }
        Ok(())
    }

    /// Downloads artifacts into the workdir at their original relative paths
    /// (all of the cargue, or only those under `dir`); returns the paths.
    pub fn fetch_into(&self, workdir: &Workdir, dir: Option<&str>) -> Result<Vec<String>> {
        let prefix = dir.map(|d| format!("{}/", d.trim_matches('/')));
        let mut fetched = Vec::new();
        for row in self.index.list(workdir.cargue_id())? {
            if prefix
                .as_ref()
                .is_some_and(|p| !row.rel_path.starts_with(p.as_str()))
            {
                continue;
            }
            self.fetch_to_file(
                workdir.cargue_id(),
                &row.rel_path,
                &workdir.file(&row.rel_path)?,
            )?;
            fetched.push(row.rel_path);
        }
        Ok(fetched)
    }

    /// Whether the artifact is published (index row; the object itself is not
    /// contacted).
    pub fn is_published(&self, cargue_id: &str, rel_path: &str) -> Result<bool> {
        validate_rel_path(rel_path)?;
        Ok(self.index.get(cargue_id, rel_path)?.is_some())
    }

    /// Index rows of the cargue, ordered by relative path.
    pub fn list(&self, cargue_id: &str) -> Result<Vec<Artefacto>> {
        Ok(self.index.list(cargue_id)?)
    }

    /// Deletes the object and its index row; false when it was not published.
    pub fn delete(&self, cargue_id: &str, rel_path: &str) -> Result<bool> {
        validate_rel_path(rel_path)?;
        let Some(row) = self.index.get(cargue_id, rel_path)? else {
            return Ok(false);
        };
        self.store
            .delete(&row.key)
            .with_context(|| format!("no se pudo borrar {rel_path}"))?;
        self.index.delete(cargue_id, rel_path)?;
        Ok(true)
    }
}

fn missing_or(err: StorageError, rel_path: &str) -> anyhow::Error {
    match err {
        StorageError::NotFound(_) => {
            anyhow::anyhow!("el artefacto {rel_path} está indexado pero falta en el bucket")
        }
        other => anyhow::Error::new(other).context(format!("no se pudo descargar {rel_path}")),
    }
}

fn verify(row: &Artefacto, sha: &str, size: i64) -> Result<()> {
    if row.sha256 != sha || row.size != size {
        bail!(
            "integridad del artefacto {}: el contenido descargado no coincide con el índice (SHA-256/tamaño)",
            row.rel_path
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::TestDb;
    use crate::storage::MemoryStore;

    fn service(db: &TestDb) -> (Artifacts, Arc<MemoryStore>) {
        let store = Arc::new(MemoryStore::new());
        (
            Artifacts::new(store.clone(), ArtefactosStore::new(db.pool())),
            store,
        )
    }

    #[test]
    fn keys_flatten_the_path_with_double_underscores() {
        assert_eq!(
            object_key("C1", "XML/FEV1_XML_PDF.zip").expect("key"),
            "C1__XML__FEV1_XML_PDF.zip"
        );
        assert_eq!(object_key("C1", "a.json").expect("key"), "C1__a.json");
        assert!(object_key("a__b", "x").is_err());
        assert!(object_key("C1", "../x").is_err());
        assert!(object_key("C1", "/x").is_err());
        assert!(object_key("C1", "a__b").is_err());
        assert!(object_key("C1", "a//b").is_err());
    }

    #[test]
    fn content_types_follow_the_extension() {
        assert_eq!(content_type_for("XML/a.XML"), "application/xml");
        assert_eq!(content_type_for("CRC/a.zip"), "application/zip");
        assert_eq!(content_type_for("noext"), "application/octet-stream");
    }

    #[test]
    fn publish_then_fetch_round_trips_and_indexes() {
        let db = TestDb::new();
        let (artifacts, store) = service(&db);
        let row = artifacts
            .publish_bytes("C1", "derechos/FEV1.json", b"{\"a\":1}")
            .expect("publish");
        assert_eq!(
            (row.key.as_str(), row.size, row.content_type.as_str()),
            ("C1__derechos__FEV1.json", 7, "application/json")
        );
        assert_eq!(store.keys(), ["C1__derechos__FEV1.json"]);
        assert_eq!(
            artifacts
                .fetch_bytes("C1", "derechos/FEV1.json")
                .expect("fetch"),
            b"{\"a\":1}"
        );
        assert!(
            artifacts
                .is_published("C1", "derechos/FEV1.json")
                .expect("published")
        );
        assert!(
            !artifacts
                .is_published("C1", "other")
                .expect("not published")
        );
        assert!(artifacts.fetch_bytes("C1", "other").is_err());
    }

    #[test]
    fn a_corrupted_object_fails_the_integrity_check() {
        let db = TestDb::new();
        let (artifacts, store) = service(&db);
        artifacts
            .publish_bytes("C1", "x.txt", b"good")
            .expect("publish");
        store
            .put_bytes("C1__x.txt", b"evil", "text/plain")
            .expect("tamper");
        let err = artifacts.fetch_bytes("C1", "x.txt").expect_err("tampered");
        assert!(err.to_string().contains("integridad"), "{err}");
        let dir = tempfile::tempdir().expect("dir");
        let dest = dir.path().join("x.txt");
        assert!(artifacts.fetch_to_file("C1", "x.txt", &dest).is_err());
        assert!(!dest.exists(), "a corrupted download is removed");
    }

    #[test]
    fn an_indexed_but_missing_object_is_reported() {
        let db = TestDb::new();
        let (artifacts, store) = service(&db);
        artifacts
            .publish_bytes("C1", "x.txt", b"good")
            .expect("publish");
        store.delete("C1__x.txt").expect("delete");
        assert!(
            artifacts
                .fetch_bytes("C1", "x.txt")
                .expect_err("missing")
                .to_string()
                .contains("falta en el bucket")
        );
    }

    #[test]
    fn workdir_round_trip_recreates_the_layout() {
        let db = TestDb::new();
        let (artifacts, _) = service(&db);
        let root = tempfile::tempdir().expect("root");
        let source = Workdir::create(root.path(), "C1").expect("workdir");
        source.write("XML/A.xml", b"<a/>").expect("a");
        source.write("CRC/z.zip", b"zip").expect("z");
        source.write("CPSCAN/A.cpscan.json", b"{}").expect("c");
        let rows = artifacts.publish_workdir(&source, None).expect("publish");
        assert_eq!(rows.len(), 3);
        drop(source);

        let other_machine = Workdir::create(root.path(), "C1").expect("workdir");
        let mut fetched = artifacts
            .fetch_into(&other_machine, Some("XML"))
            .expect("fetch xml");
        assert_eq!(fetched, ["XML/A.xml"]);
        fetched = artifacts
            .fetch_into(&other_machine, None)
            .expect("fetch all");
        assert_eq!(fetched.len(), 3);
        assert_eq!(other_machine.read("CRC/z.zip").expect("read"), b"zip");
        assert!(artifacts.list("C2").expect("other cargue").is_empty());
    }

    #[test]
    fn republishing_overwrites_and_delete_removes_both() {
        let db = TestDb::new();
        let (artifacts, store) = service(&db);
        artifacts.publish_bytes("C1", "a.txt", b"one").expect("v1");
        let v2 = artifacts
            .publish_bytes("C1", "a.txt", b"three")
            .expect("v2");
        assert_eq!(v2.size, 5);
        assert_eq!(
            artifacts.fetch_bytes("C1", "a.txt").expect("fetch"),
            b"three"
        );
        assert!(artifacts.delete("C1", "a.txt").expect("delete"));
        assert!(!artifacts.delete("C1", "a.txt").expect("again"));
        assert!(store.is_empty());
    }
}
