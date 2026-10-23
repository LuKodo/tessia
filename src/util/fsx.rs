//! File-system primitives: atomic writes, private files and moves that work
//! across volumes.
//!
//! Every file this program creates is private to the owner (mode `0600` on
//! Unix; directories `0700`) because workdirs hold patient data.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Creates `dir` and its parents (mode `0700` on Unix).
pub fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("no se pudo crear la carpeta {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best effort: an existing shared parent keeps its mode.
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn open_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Writes `data` to `path` (created with mode `0600`, truncated if present),
/// creating parent directories first. Not atomic; see [`atomic_write`].
pub fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let mut file =
        open_private(path).with_context(|| format!("no se pudo escribir {}", path.display()))?;
    file.write_all(data)
        .with_context(|| format!("no se pudo escribir {}", path.display()))
}

/// Writes `data` to a temporary file next to `path`, syncs it and renames it
/// over `path`, so a reader never observes partial content. Mode `0600`.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    ensure_dir(parent)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{name}.tmp-{}", uuid::Uuid::new_v4().simple()));
    let result = (|| -> io::Result<()> {
        let mut file = open_private(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if let Err(err) = result {
        let _ = fs::remove_file(&tmp);
        return Err(err).with_context(|| format!("no se pudo escribir {}", path.display()));
    }
    Ok(())
}

/// Renames `src` to `dst` (replacing it), falling back to copy + remove when
/// both are on different volumes.
pub fn move_file(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        ensure_dir(parent)?;
    }
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices => {
            fs::copy(src, dst).with_context(|| {
                format!("no se pudo copiar {} a {}", src.display(), dst.display())
            })?;
            fs::remove_file(src)
                .with_context(|| format!("copiado, pero no se pudo borrar {}", src.display()))
        }
        Err(err) => Err(err)
            .with_context(|| format!("no se pudo mover {} a {}", src.display(), dst.display())),
    }
}

/// Joins `rel` under `base`, refusing absolute paths and any `..` component.
pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let mut out = base.to_path_buf();
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => bail!("ruta insegura: {rel}"),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a/b.txt");
        atomic_write(&path, b"one").expect("first write");
        atomic_write(&path, b"two").expect("second write");
        assert_eq!(fs::read(&path).expect("read"), b"two");
        let entries = fs::read_dir(path.parent().expect("parent"))
            .expect("list")
            .count();
        assert_eq!(entries, 1);
    }

    #[cfg(unix)]
    #[test]
    fn written_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("x");
        write_private(&path, b"x").expect("write");
        let mode = fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn move_file_replaces_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (src, dst) = (dir.path().join("s"), dir.path().join("d/t"));
        fs::write(&src, b"new").expect("src");
        move_file(&src, &dst).expect("move");
        assert!(!src.exists());
        assert_eq!(fs::read(&dst).expect("dst"), b"new");
    }

    #[test]
    fn safe_join_rejects_traversal() {
        let base = Path::new("/w");
        assert!(safe_join(base, "../x").is_err());
        assert!(safe_join(base, "/etc").is_err());
        assert_eq!(
            safe_join(base, "XML/a.xml").expect("ok"),
            Path::new("/w/XML/a.xml")
        );
    }
}
