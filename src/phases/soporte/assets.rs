//! Shared state of the delivery page: the patient signature files and the logo.
//!
//! In Go these were machine-local folders (`paths.firmas`, `paths.reportes`).
//! With N machines they live in the bucket, as artifacts of the reserved
//! cargue id [`SHARED_CARGUE`] (documented in `docs/architecture.md`):
//!
//! * signature of a patient: `firmas/<tipodoc><numerodoc>.png`
//!   (flat key `_shared__firmas__<tipodoc><numerodoc>.png`);
//! * logo of the delivery page: `reportes/logo.png`.
//!
//! Lookup order (first hit wins):
//!
//! 1. the shared artifact;
//! 2. the legacy local folder, read only: `paths.firmas/<name>` and
//!    `paths.reportes/logo.png` (so an existing folder keeps working on the
//!    machine that has it, without uploading it first);
//! 3. the logo only: the synthetic placeholder embedded in the binary
//!    (`assets/soporte/logo.png`); put the real logo in the bucket or in
//!    `paths.reportes`. A signature that is nowhere is drawn from the patient
//!    name and published to the shared artifacts without overwriting.

use std::path::PathBuf;

use anyhow::{Result, anyhow};

use super::generator::ManualSignature;
use super::input::Header;
use super::render::image::{self, Accept};
use super::render::signature;
use crate::context::AppContext;
use crate::storage::validate_rel_path;

/// Reserved cargue id holding the files shared by every cargue.
pub const SHARED_CARGUE: &str = "_shared";

/// A signature file shorter than this is not valid (the soporte flow requires >= 10 bytes).
pub const MIN_SIGNATURE_BYTES: usize = 10;

/// The logo shipped with the binary (synthetic placeholder).
const EMBEDDED_LOGO: &[u8] = include_bytes!("../../../assets/soporte/logo.png");

/// Artifact path of the logo.
pub const LOGO_REL: &str = "reportes/logo.png";

/// `<tipodoc><numerodoc>.png`, or `None` when the document would not make a
/// plain, bucket-safe file name.
pub fn signature_file(doc_type: &str, doc_number: &str) -> Option<String> {
    let name = format!("{}{}.png", doc_type.trim(), doc_number.trim());
    let safe = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    let valid = name != ".png" && !name.contains("..") && !name.contains("__") && safe;
    valid.then_some(name)
}

/// Artifact path of a signature file name.
pub fn signature_rel(file: &str) -> String {
    format!("firmas/{file}")
}

/// What a lookup of a stored file found.
enum Found {
    Valid(Vec<u8>),
    /// A file exists but is too small (never replaced).
    Invalid,
    Missing,
}

/// The shared artifact or, failing that, the legacy local file.
fn find(ctx: &AppContext, rel: &str, local: Option<PathBuf>, min: usize) -> Result<Found> {
    validate_rel_path(rel)?;
    let mut invalid = false;
    if ctx.artefactos().get(SHARED_CARGUE, rel)?.is_some() {
        match ctx.artifacts().fetch_bytes(SHARED_CARGUE, rel) {
            Ok(bytes) if bytes.len() >= min => return Ok(Found::Valid(bytes)),
            Ok(_) => invalid = true,
            Err(err) => {
                tracing::warn!(rel, error = %err, "shared file unreadable");
                invalid = true;
            }
        }
    }
    if let Some(path) = local.filter(|p| p.is_file()) {
        match std::fs::read(&path) {
            Ok(bytes) if bytes.len() >= min => return Ok(Found::Valid(bytes)),
            _ => invalid = true,
        }
    }
    Ok(if invalid {
        Found::Invalid
    } else {
        Found::Missing
    })
}

fn local_dir(ctx: &AppContext, key: &str) -> Option<PathBuf> {
    ctx.settings
        .get(key)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The signature file of the patient if it exists and is valid (never generated):
/// what the CPScan-backed delivery page may use.
pub fn existing_signature(ctx: &AppContext, header: &Header) -> Result<Option<Vec<u8>>> {
    let Some(file) = signature_file(&header.doc_type, &header.doc_number) else {
        return Ok(None);
    };
    let local = local_dir(ctx, "paths.firmas").map(|d| d.join(&file));
    Ok(
        match find(ctx, &signature_rel(&file), local, MIN_SIGNATURE_BYTES)? {
            Found::Valid(bytes) => Some(bytes),
            Found::Invalid | Found::Missing => None,
        },
    )
}

/// `asegurarFirma`: the patient's file when valid, else a signature drawn from
/// the name, published to the shared artifacts without overwriting anything.
pub fn manual_signature(ctx: &AppContext, header: &Header) -> Result<ManualSignature> {
    let file = signature_file(&header.doc_type, &header.doc_number);
    let mut existing = false;
    if let Some(file) = &file {
        let local = local_dir(ctx, "paths.firmas").map(|d| d.join(file));
        match find(ctx, &signature_rel(file), local, MIN_SIGNATURE_BYTES)? {
            Found::Valid(png) => {
                return Ok(ManualSignature {
                    png,
                    generated: false,
                    note: String::new(),
                });
            }
            Found::Invalid => existing = true,
            Found::Missing => {}
        }
    }
    let png = signature::render(&header.name).map_err(|e| anyhow!("generar firma: {e:#}"))?;
    let note = match &file {
        None => "firma generada sin guardar (tipo/numero de documento no validos como nombre de archivo)".to_owned(),
        Some(_) if existing => {
            "firma generada sin guardar (el archivo existente no es valido y no se reemplaza)".to_owned()
        }
        Some(file) => publish_new(ctx, &signature_rel(file), &png),
    };
    Ok(ManualSignature {
        png,
        generated: true,
        note,
    })
}

/// Publishes a generated signature unless someone published one meanwhile;
/// returns the note ("" when saved).
fn publish_new(ctx: &AppContext, rel: &str, png: &[u8]) -> String {
    let artifacts = ctx.artifacts();
    match artifacts.is_published(SHARED_CARGUE, rel) {
        Ok(false) => match artifacts.publish_bytes(SHARED_CARGUE, rel, png) {
            Ok(_) => String::new(),
            Err(err) => format!("firma generada sin guardar: {err:#}"),
        },
        Ok(true) => "firma generada sin guardar (otra maquina la guardo antes)".to_owned(),
        Err(err) => format!("firma generada sin guardar: {err:#}"),
    }
}

/// The logo of the delivery page, validated as PNG or JPEG. An error is kept
/// as text: only the invoices whose delivery page is rendered fail with it.
pub fn load_logo(ctx: &AppContext) -> std::result::Result<Vec<u8>, String> {
    let local = local_dir(ctx, "paths.reportes").map(|d| d.join("logo.png"));
    let data = match find(ctx, LOGO_REL, local, 1) {
        Ok(Found::Valid(bytes)) => bytes,
        Ok(Found::Invalid) => return Err("supportrenderer: delivery logo is empty".into()),
        Ok(Found::Missing) => EMBEDDED_LOGO.to_vec(),
        Err(err) => return Err(format!("supportrenderer: delivery logo: {err:#}")),
    };
    image::probe(&data, Accept::PngJpeg)
        .map_err(|_| "supportrenderer: delivery logo is not a PNG or JPEG image".to_owned())?;
    Ok(data)
}
