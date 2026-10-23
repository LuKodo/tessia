//! Per-cargue temp working directory.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::util::fsx;

/// Folder names of the original Go layout, relative to the workdir root.
/// Phase writers add their own constants when they need another folder.
pub mod layout {
    /// Invoice XML, PDF and ZIP XML/PDF (`XML/<F>_XML_PDF.zip`).
    pub const XML: &str = "XML";
    /// CRC support ZIPs (`CRC/CRC_<nit>_<F>.zip`).
    pub const CRC: &str = "CRC";
    /// CPScan support files and sidecars (`CPSCAN/<F>.support.pdf`, `<F>.cpscan.json`).
    pub const CPSCAN: &str = "CPSCAN";
    /// Rights per invoice (`derechos/<F>.json`).
    pub const DERECHOS: &str = "derechos";
    /// Filed files archive (`Radicado/<relative path>`).
    pub const RADICADO: &str = "Radicado";
}

/// A temporary directory of one cargue on this machine, recreating the original
/// folder layout. It is created under `workdir.root` with mode `0700`, every
/// file written through it is `0600`, and it removes itself (recursively) when
/// dropped: nothing about a cargue stays on disk after a phase ends.
///
/// ```
/// use tessia::storage::{Workdir, layout};
///
/// # fn demo() -> anyhow::Result<()> {
/// let root = tempfile::tempdir()?;
/// let wd = Workdir::create(root.path(), "Convenio_21102026_21102026")?;
/// wd.write("XML/FEV1.xml", b"<xml/>")?;
/// assert_eq!(wd.files()?, ["XML/FEV1.xml"]);
/// let path = wd.path().to_path_buf();
/// drop(wd);
/// assert!(!path.exists());
/// # Ok(()) }
/// # demo().unwrap();
/// ```
#[derive(Debug)]
pub struct Workdir {
    path: PathBuf,
    cargue_id: String,
    keep: bool,
}

impl Workdir {
    /// Creates `<root>/<cargue_id>-<random>/`.
    pub fn create(root: &Path, cargue_id: &str) -> Result<Workdir> {
        let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let path = root.join(format!("{cargue_id}-{suffix}"));
        fsx::ensure_dir(&path)?;
        Ok(Workdir {
            path,
            cargue_id: cargue_id.to_owned(),
            keep: false,
        })
    }

    /// The directory itself.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn cargue_id(&self) -> &str {
        &self.cargue_id
    }

    /// Absolute path of `rel` (`/`-separated, no `..`), creating its parents.
    pub fn file(&self, rel: &str) -> Result<PathBuf> {
        let path = fsx::safe_join(&self.path, rel)?;
        if let Some(parent) = path.parent() {
            fsx::ensure_dir(parent)?;
        }
        Ok(path)
    }

    /// Absolute path of the sub-directory `rel`, created if missing.
    pub fn dir(&self, rel: &str) -> Result<PathBuf> {
        let path = fsx::safe_join(&self.path, rel)?;
        fsx::ensure_dir(&path)?;
        Ok(path)
    }

    /// Writes `data` to `rel` (mode `0600`, atomically) and returns its path.
    pub fn write(&self, rel: &str, data: &[u8]) -> Result<PathBuf> {
        let path = self.file(rel)?;
        fsx::atomic_write(&path, data)?;
        Ok(path)
    }

    pub fn read(&self, rel: &str) -> Result<Vec<u8>> {
        let path = fsx::safe_join(&self.path, rel)?;
        std::fs::read(&path).with_context(|| format!("no se pudo leer {rel}"))
    }

    pub fn exists(&self, rel: &str) -> bool {
        fsx::safe_join(&self.path, rel)
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Relative paths (`/`-separated, sorted) of every file in the workdir.
    pub fn files(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        collect(&self.path, &self.path, &mut out)?;
        out.sort();
        Ok(out)
    }

    /// Relative paths of the files under the sub-directory `dir`.
    pub fn files_in(&self, dir: &str) -> Result<Vec<String>> {
        let prefix = format!("{}/", dir.trim_matches('/'));
        Ok(self
            .files()?
            .into_iter()
            .filter(|f| f.starts_with(&prefix))
            .collect())
    }

    /// Keeps the directory on drop (debugging) and returns its path.
    pub fn keep(mut self) -> PathBuf {
        self.keep = true;
        self.path.clone()
    }
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in
        std::fs::read_dir(dir).with_context(|| format!("no se pudo listar {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            let parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            out.push(parts.join("/"));
        }
    }
    Ok(())
}

impl Drop for Workdir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recreates_layout_lists_files_and_cleans_up() {
        let root = tempfile::tempdir().expect("root");
        let wd = Workdir::create(root.path(), "C1").expect("workdir");
        let path = wd.path().to_path_buf();
        wd.write("XML/B.xml", b"b").expect("b");
        wd.write("XML/A.xml", b"a").expect("a");
        wd.write("CRC/z.zip", b"z").expect("z");
        wd.dir("CPSCAN").expect("empty dir");
        assert_eq!(
            wd.files().expect("files"),
            ["CRC/z.zip", "XML/A.xml", "XML/B.xml"]
        );
        assert_eq!(wd.files_in("XML").expect("xml"), ["XML/A.xml", "XML/B.xml"]);
        assert!(wd.exists("XML/A.xml") && !wd.exists("XML/none.xml"));
        assert_eq!(wd.read("XML/A.xml").expect("read"), b"a");
        drop(wd);
        assert!(!path.exists());
    }

    #[test]
    fn paths_cannot_escape_the_workdir() {
        let root = tempfile::tempdir().expect("root");
        let wd = Workdir::create(root.path(), "C1").expect("workdir");
        assert!(wd.write("../evil", b"x").is_err());
        assert!(wd.file("/etc/passwd").is_err());
        assert!(!wd.exists("../x"));
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_and_its_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("root");
        let wd = Workdir::create(root.path(), "C1").expect("workdir");
        let file = wd.write("XML/a.xml", b"a").expect("write");
        let mode = |p: &Path| std::fs::metadata(p).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(wd.path()), 0o700);
    }

    #[test]
    fn keep_disarms_the_cleanup() {
        let root = tempfile::tempdir().expect("root");
        let path = Workdir::create(root.path(), "C1").expect("workdir").keep();
        assert!(path.exists());
    }
}
