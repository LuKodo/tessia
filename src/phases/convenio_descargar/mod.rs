//! `convenio descargar`: downloads the convenio CSV from Medicar and publishes
//! it as an artifact of its cargue.
//!
//! Differences from Go (new design): there is no `paths.automatic` folder and
//! no `.origin` sidecar. The CSV is downloaded to a temp file in the
//! per-machine [`Workdir`](crate::storage::Workdir), registered as a cargue
//! (id = file stem, origin `medicar`, SHA-256) and published as the artifact
//! `Convenio_ddMMyyyy_ddMMyyyy.csv`. A lease named after the cargue keeps two
//! machines from downloading the same range at once.

pub mod cli;
pub mod client;
#[cfg(any(test, feature = "test-support"))]
pub mod mock;

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Datelike, Local, NaiveDate};
use sha2::{Digest, Sha256};

use crate::context::AppContext;
use crate::domain::cargue::{LoadOrigin, load_context};
use crate::stores::cargues::NewCargue;

pub use cli::Args;
pub use client::{MedicarClient, MedicarOptions, export_fields, file_name};

/// How long a download holds its lease without a renewal.
const LEASE_TTL: Duration = Duration::from_secs(10 * 60);
/// The lease is renewed this often while the download runs.
const LEASE_RENEW_EVERY: Duration = Duration::from_secs(2 * 60);

/// What publishing the downloaded CSV did to the cargue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    /// First upload of this cargue id.
    Created,
    /// The same bytes were already registered.
    Unchanged,
    /// Different bytes under an id that already had uploads.
    NewUpload,
}

impl DownloadStatus {
    /// Spanish description printed after the saved line.
    pub fn describe(self) -> &'static str {
        match self {
            DownloadStatus::Created => "cargue nuevo",
            DownloadStatus::Unchanged => "sin cambios: los mismos bytes ya estaban registrados",
            DownloadStatus::NewUpload => {
                "nueva carga bajo el mismo cargue: el contenido cambió respecto a la anterior"
            }
        }
    }
}

/// Result of a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOutcome {
    /// Cargue id (file stem).
    pub cargue_id: String,
    /// Artifact path inside the cargue (`Convenio_ddMMyyyy_ddMMyyyy.csv`).
    pub file_name: String,
    /// Bucket object name.
    pub object_key: String,
    /// Lowercase hex SHA-256 of the CSV.
    pub sha256: String,
    pub size: u64,
    pub status: DownloadStatus,
}

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    let today = Local::now().date_naive();
    let (from, to) = resolve_range(
        args.desde.as_deref(),
        args.hasta.as_deref(),
        args.actual,
        today,
    )?;
    let outcome = download_range(ctx, from, to)?;
    println!(
        "convenio saved: {} ({})",
        outcome.cargue_id, outcome.object_key
    );
    println!("{}", outcome.status.describe());
    Ok(())
}

/// Go's date rules: `--actual` (first of the month through `today`) excludes
/// `--desde/--hasta`; dates are `YYYY-MM-DD`; the end may not precede the start.
pub fn resolve_range(
    desde: Option<&str>,
    hasta: Option<&str>,
    actual: bool,
    today: NaiveDate,
) -> Result<(NaiveDate, NaiveDate)> {
    let given = |v: Option<&str>| v.is_some_and(|v| !v.is_empty());
    let (from, to) = if actual {
        if given(desde) || given(hasta) {
            bail!("use either --actual or --desde/--hasta");
        }
        let first = today
            .with_day(1)
            .ok_or_else(|| anyhow!("fecha actual inválida"))?;
        (first, today)
    } else {
        (
            parse_day("desde", desde.unwrap_or_default())?,
            parse_day("hasta", hasta.unwrap_or_default())?,
        )
    };
    if to < from {
        bail!("--hasta is before --desde");
    }
    Ok((from, to))
}

fn parse_day(flag: &str, value: &str) -> Result<NaiveDate> {
    let text = value.trim();
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .filter(|date| date.format("%Y-%m-%d").to_string() == text)
        .ok_or_else(|| anyhow!("--{flag} must be YYYY-MM-DD"))
}

/// Downloads the convenio of `[from, to]` (the `medicar.*` settings), registers
/// it as a cargue and publishes it. Used by the CLI and by the daily entry point.
pub fn download_range(ctx: &AppContext, from: NaiveDate, to: NaiveDate) -> Result<DownloadOutcome> {
    let options = MedicarOptions::from_settings(&ctx.settings)?;
    download_range_with(ctx, from, to, options)
}

/// [`download_range`] with explicit client options (tests use tiny backoffs).
pub fn download_range_with(
    ctx: &AppContext,
    from: NaiveDate,
    to: NaiveDate,
    options: MedicarOptions,
) -> Result<DownloadOutcome> {
    if to < from {
        bail!("medicar: the end date is before the start date");
    }
    let client = MedicarClient::new(options)?;
    let name = file_name(from, to);
    let mut info = load_context(&name);
    info.origin = LoadOrigin::Medicar;
    let lease = format!("convenio_descargar:{}", info.id);
    with_lease(ctx, &lease, &info.id, || {
        let workdir = ctx.workdir(&info.id)?;
        let csv = workdir.file(&name)?;
        client.download(from, to, &csv)?;
        publish(ctx, &info, &csv)
    })
}

/// Registers the cargue (idempotent by SHA-256) and publishes the artifact.
/// The cargue is registered first so a superseded upload fails before the
/// published artifact is replaced; a failed publish is healed by re-running.
fn publish(
    ctx: &AppContext,
    info: &crate::domain::cargue::LoadInfo,
    csv: &Path,
) -> Result<DownloadOutcome> {
    let (sha256, size) = sha256_file(csv)?;
    let previous = ctx.cargues().newest(&info.id)?;
    let ingested = ctx.cargues().ingest(&NewCargue::from_load(info, &sha256))?;
    let artefacto = ctx
        .artifacts()
        .publish_file(&info.id, &info.name, csv)
        .context("no se pudo publicar el CSV descargado")?;
    let status = match (ingested.created, previous) {
        (false, _) => DownloadStatus::Unchanged,
        (true, None) => DownloadStatus::Created,
        (true, Some(_)) => DownloadStatus::NewUpload,
    };
    Ok(DownloadOutcome {
        cargue_id: info.id.clone(),
        file_name: info.name.clone(),
        object_key: artefacto.key,
        sha256,
        size,
        status,
    })
}

fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let mut file =
        File::open(path).with_context(|| format!("no se pudo leer {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((hex::encode(hasher.finalize()), size))
}

/// Runs `work` holding the lease `name`, renewed in the background; fails
/// with a Spanish message when another owner holds it.
fn with_lease<T: Send>(
    ctx: &AppContext,
    name: &str,
    cargue_id: &str,
    work: impl FnOnce() -> Result<T> + Send,
) -> Result<T> {
    let leases = ctx.leases();
    let Some(guard) = leases.guard(name, &ctx.owner, LEASE_TTL)? else {
        let holder = leases.holder(name).ok().flatten().unwrap_or_default();
        bail!(
            "otra máquina ya está descargando el cargue {cargue_id} (lease «{name}» en poder de {holder}); espere a que termine o a que el lease expire"
        );
    };
    let (stop, stopped) = crossbeam_channel::bounded::<()>(1);
    std::thread::scope(|scope| {
        let heartbeat = scope.spawn(|| {
            while let Err(crossbeam_channel::RecvTimeoutError::Timeout) =
                stopped.recv_timeout(LEASE_RENEW_EVERY)
            {
                match guard.renew() {
                    Ok(true) => {}
                    Ok(false) => tracing::warn!(lease = %name, "the download lease was lost"),
                    Err(err) => {
                        tracing::warn!(lease = %name, error = %err, "could not renew the lease")
                    }
                }
            }
        });
        let result = work();
        drop(stop);
        let _ = heartbeat.join();
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("date")
    }

    const TODAY: (i32, u32, u32) = (2026, 10, 22);

    fn range(
        desde: Option<&str>,
        hasta: Option<&str>,
        actual: bool,
    ) -> Result<(NaiveDate, NaiveDate)> {
        resolve_range(desde, hasta, actual, day(TODAY.0, TODAY.1, TODAY.2))
    }

    #[test]
    fn explicit_dates_are_parsed() {
        let (from, to) = range(Some("2026-10-01"), Some(" 2026-10-21 "), false).expect("range");
        assert_eq!((from, to), (day(2026, 10, 1), day(2026, 10, 21)));
        assert!(range(Some("2026-10-05"), Some("2026-10-05"), false).is_ok());
    }

    #[test]
    fn actual_is_the_first_of_the_month_through_today() {
        let (from, to) = range(None, None, true).expect("actual");
        assert_eq!((from, to), (day(2026, 10, 1), day(2026, 10, 22)));
        let (from, to) =
            resolve_range(Some(""), Some(""), true, day(2026, 10, 1)).expect("empty flags");
        assert_eq!((from, to), (day(2026, 10, 1), day(2026, 10, 1)));
    }

    #[test]
    fn actual_conflicts_with_desde_and_hasta() {
        for (desde, hasta) in [
            (Some("2026-10-01"), None),
            (None, Some("2026-10-02")),
            (Some("2026-10-01"), Some("2026-10-02")),
        ] {
            let err = range(desde, hasta, true).expect_err("conflict");
            assert_eq!(err.to_string(), "use either --actual or --desde/--hasta");
        }
    }

    #[test]
    fn bad_or_missing_dates_name_the_flag() {
        for (desde, hasta, flag) in [
            (None, Some("2026-10-02"), "desde"),
            (Some("2026-10-02"), None, "hasta"),
            (Some("02/10/2026"), Some("2026-10-02"), "desde"),
            (Some("2026-10-02"), Some("2026-13-01"), "hasta"),
            (Some("2026-1-2"), Some("2026-10-02"), "desde"),
            (Some("2026-02-30"), Some("2026-10-02"), "desde"),
        ] {
            let err = range(desde, hasta, false).expect_err("invalid");
            assert_eq!(
                err.to_string(),
                format!("--{flag} must be YYYY-MM-DD"),
                "{desde:?} {hasta:?}"
            );
        }
    }

    #[test]
    fn end_before_start_is_an_error() {
        let err = range(Some("2026-10-05"), Some("2026-10-04"), false).expect_err("order");
        assert_eq!(err.to_string(), "--hasta is before --desde");
    }
}
