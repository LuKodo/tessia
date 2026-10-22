//! Where the CSV of a preparation comes from (Go `IngestCargue` +
//! `CargueLines`, adapted to the bucket, S5/S8).
//!
//! The positional argument is either
//!
//! * a **local CSV file**: registered as a cargue exactly like Go
//!   (`id` = the folder-safe file stem, SHA-256 idempotent, an older upload of
//!   a load whose newest upload differs fails with `Superseded`) and
//!   published to the bucket as the artifact `<file name>`, so any machine can
//!   re-read it; or
//! * the **id of an existing cargue**: its CSV artifact is read back from the
//!   bucket (the newest upload; its SHA-256 must match the cargue row).
//!
//! A file path wins over an id of the same spelling. Origin of a local file:
//! the `<file>.origin` sidecar when present (`manual`, `medicar`, `unknown`),
//! else `medicar` for a name that looks like a Medicar download
//! (`Convenio_ddMMyyyy_ddMMyyyy...`) and `manual` for anything else.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

use crate::context::AppContext;
use crate::domain::cargue::{LoadInfo, LoadOrigin, is_valid_id, load_context};
use crate::storage::validate_rel_path;
use crate::stores::cargues::{Cargue, NewCargue};

/// The CSV of a preparation, read but not yet registered.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub info: LoadInfo,
    pub sha256: String,
    pub bytes: Vec<u8>,
    /// `Some(seq)` for an existing cargue; `None` for a local file that must
    /// still be registered ([`register`]).
    pub cargue: Option<i64>,
}

/// The CSV of a preparation and the cargue it belongs to.
#[derive(Debug, Clone)]
pub struct LoadedCsv {
    pub info: LoadInfo,
    /// `cargues.seq`: the key of every dataset call.
    pub cargue: i64,
    pub sha256: String,
    pub bytes: Vec<u8>,
    /// False when the same bytes were already registered.
    pub created: bool,
}

/// Reads the CSV of `argument` (file path or cargue id) without registering
/// anything.
pub fn resolve_source(ctx: &AppContext, argument: &str) -> Result<Resolved> {
    let argument = argument.trim();
    if argument.is_empty() {
        bail!("falta el CSV o el cargue a preparar");
    }
    let path = Path::new(argument);
    if path.is_file() {
        return read_local(path);
    }
    if is_valid_id(argument) {
        return read_existing(ctx, argument);
    }
    Err(anyhow!(
        "no existe el archivo {argument:?} ni es un id de cargue válido"
    ))
}

/// Registers a resolved CSV as a cargue (idempotent by SHA-256) and makes sure
/// the bucket holds it.
pub fn register(ctx: &AppContext, resolved: Resolved) -> Result<LoadedCsv> {
    let Resolved {
        info,
        sha256,
        bytes,
        cargue,
    } = resolved;
    if let Some(cargue) = cargue {
        return Ok(LoadedCsv {
            info,
            cargue,
            sha256,
            bytes,
            created: false,
        });
    }
    // Registered first: a superseded upload fails before the artifact is replaced.
    let ingested = ctx
        .cargues()
        .ingest(&NewCargue::from_load(&info, &sha256))
        .context("no se pudo registrar el cargue")?;
    let artifacts = ctx.artifacts();
    if ingested.created || !artifacts.is_published(&info.id, &info.name)? {
        artifacts
            .publish_bytes(&info.id, &info.name, &bytes)
            .context("no se pudo subir el CSV al bucket")?;
    }
    Ok(LoadedCsv {
        info,
        cargue: ingested.cargue.seq,
        sha256,
        bytes,
        created: ingested.created,
    })
}

/// [`resolve_source`] followed by [`register`].
pub fn load_source(ctx: &AppContext, argument: &str) -> Result<LoadedCsv> {
    register(ctx, resolve_source(ctx, argument)?)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Origin of a local file (see the module docs).
pub fn origin_of(path: &Path, info: &LoadInfo) -> Result<LoadOrigin> {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(".origin");
    match std::fs::read_to_string(&sidecar) {
        Ok(text) => LoadOrigin::parse(&text)
            .ok_or_else(|| anyhow!("convenio: invalid load origin {:?}", text.trim())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let medicar_name = info.name.starts_with("Convenio_") && info.from.is_some();
            Ok(if medicar_name {
                LoadOrigin::Medicar
            } else {
                LoadOrigin::Manual
            })
        }
        Err(err) => Err(err).context("convenio: read load origin"),
    }
}

fn read_local(path: &Path) -> Result<Resolved> {
    let bytes =
        std::fs::read(path).with_context(|| format!("no se pudo leer {}", path.display()))?;
    let mut info = load_context(&path.to_string_lossy());
    info.origin = origin_of(path, &info)?;
    if validate_rel_path(&info.name).is_err() {
        info.name = format!("{}.csv", info.id);
    }
    Ok(Resolved {
        sha256: sha256_hex(&bytes),
        info,
        bytes,
        cargue: None,
    })
}

fn read_existing(ctx: &AppContext, id: &str) -> Result<Resolved> {
    let cargue = ctx.cargues().newest(id)?.ok_or_else(|| {
        anyhow!("no existe el cargue «{id}»: pase la ruta de un CSV o descárguelo primero")
    })?;
    let bytes = ctx
        .artifacts()
        .fetch_bytes(&cargue.id, &cargue.name)
        .with_context(|| {
            format!(
                "no se pudo leer el CSV «{}» del cargue «{id}» del bucket",
                cargue.name
            )
        })?;
    let sha256 = sha256_hex(&bytes);
    if sha256 != cargue.sha256 {
        bail!(
            "el CSV publicado del cargue «{id}» no coincide con su registro más reciente (sha256 {} frente a {}): vuelva a descargarlo o a subir el archivo",
            &sha256[..12],
            &cargue.sha256[..12]
        );
    }
    Ok(Resolved {
        info: info_of(&cargue),
        cargue: Some(cargue.seq),
        sha256,
        bytes,
    })
}

fn info_of(cargue: &Cargue) -> LoadInfo {
    LoadInfo {
        name: cargue.name.clone(),
        id: cargue.id.clone(),
        from: cargue.date_from,
        to: cargue.date_to,
        origin: cargue.origin(),
    }
}
