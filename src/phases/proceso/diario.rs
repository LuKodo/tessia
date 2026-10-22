//! `facturacion diario`: the whole process for one day, meant for a daily cron
//! on any machine (S3).
//!
//! 1. The day is yesterday in local time, or `--fecha`.
//! 2. **Download.** The cargue `Convenio_ddMMyyyy_ddMMyyyy` of that day is
//!    reused when it is already registered with its CSV published (by an
//!    earlier run, another machine or a manual download); otherwise
//!    `convenio descargar` runs for `[day, day]` (`--redescargar` forces it).
//!    Two machines downloading the same day are serialized by the lease of
//!    `convenio_descargar`; the loser waits for the winner and reuses its CSV.
//! 3. **Prepare + process.** [`execute`] runs `proceso run` on that cargue:
//!    it prepares the cargue only when it is not prepared yet (never twice:
//!    re-preparing would reset invoices, see [`super::seed`]) and then streams
//!    the invoices through the phases, claiming each (invoice, phase), so any
//!    number of cron or manual machines split the work without colliding. No
//!    lease spans the processing.
//! 4. A short Spanish summary is printed. The command exits non-zero when the
//!    run failed or any invoice ended `FAILED` (or a radicación batch
//!    failed); manual review and excluded invoices are data, not failures.
//!
//! Running it twice for the same day is safe: the second run finds the CSV,
//! the dataset and every finished phase in place and only picks up what is
//! left (invoices already filed are skipped).

use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Duration as Days, Local, NaiveDate};
use clap::Args as ClapArgs;

use super::cli::RunFlags;
use super::config::RunConfig;
use super::events::{Events, Mode};
use super::run::{RunSummary, execute};
use super::runner::PhaseRunner;
use crate::context::AppContext;
use crate::domain::cargue::load_context;
use crate::phases::convenio_descargar::{self, DownloadOutcome, file_name};

/// Procesa de punta a punta el día anterior (o --fecha): descarga el convenio
/// de Medicar, lo prepara y corre `proceso run` sobre el cargue. Pensado para
/// un cron diario en cualquier máquina; es idempotente y se puede ejecutar
/// mientras otras máquinas procesan el mismo cargue a mano (cada factura se
/// reclama por fase, así que el trabajo se reparte). Termina con código distinto
/// de cero si la corrida falló o alguna factura quedó FAILED.
#[derive(Debug, Clone, ClapArgs)]
#[command(override_usage = "facturacion diario [--fecha AAAA-MM-DD] [opciones]")]
pub struct Args {
    /// Día a procesar AAAA-MM-DD, hora local (por defecto ayer)
    #[arg(long, value_name = "FECHA")]
    pub fecha: Option<String>,

    /// Vuelve a descargar el CSV de Medicar aunque el cargue ya exista
    #[arg(long)]
    pub redescargar: bool,

    /// Pool de hilos de todas las fases (por defecto el de cada fase)
    #[arg(long, value_name = "N")]
    pub workers: Option<usize>,

    #[command(flatten)]
    pub flags: RunFlags,
}

/// How the CSV of the day was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Csv {
    Downloaded,
    Reused,
}

/// What the day's run did.
#[derive(Debug, Clone)]
pub struct DiarioSummary {
    pub date: NaiveDate,
    pub cargue_id: String,
    pub csv: Csv,
    pub run: RunSummary,
}

impl DiarioSummary {
    /// True when the cron should alert: a failed invoice or radicación batch.
    pub fn failed(&self) -> bool {
        self.run.finished.failed > 0
    }

    /// The short Spanish summary.
    pub fn lines(&self) -> Vec<String> {
        let f = &self.run.finished;
        let csv = match self.csv {
            Csv::Downloaded => "descargado",
            Csv::Reused => "reutilizado",
        };
        let prepared = if self.run.prepared_now {
            "preparado ahora"
        } else {
            "ya estaba preparado"
        };
        let mut lines = vec![
            format!(
                "diario {}: cargue {} (CSV {csv}, {prepared})",
                self.date, self.cargue_id
            ),
            format!(
                "facturas: {} procesadas, {} terminadas, {} en revisión manual, {} excluidas, {} fallidas, {} omitidas (otra máquina)",
                f.entered, f.done, f.manual, f.excluded, f.failed, f.skipped
            ),
        ];
        if f.replaced > 0 {
            lines.push(format!("reemplazos: {}", f.replaced));
        }
        lines
    }
}

/// The day to process: `--fecha`, else yesterday. A future day is an error.
pub fn resolve_date(fecha: Option<&str>, today: NaiveDate) -> Result<NaiveDate> {
    let Some(text) = fecha.map(str::trim).filter(|t| !t.is_empty()) else {
        return today
            .checked_sub_signed(Days::days(1))
            .ok_or_else(|| anyhow!("fecha actual inválida"));
    };
    let day = NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .filter(|d| d.format("%Y-%m-%d").to_string() == text)
        .ok_or_else(|| anyhow!("--fecha must be YYYY-MM-DD"))?;
    if day > today {
        bail!("--fecha {day} es una fecha futura");
    }
    Ok(day)
}

/// Downloads `[day, day]` (the real wiring is `convenio_descargar::download_range`).
pub type DownloadFn<'a> = &'a (dyn Fn(&AppContext, NaiveDate) -> Result<DownloadOutcome> + Sync);

/// The cargue of a day and its CSV artifact name.
pub fn cargue_of(day: NaiveDate) -> (String, String) {
    let name = file_name(day, day);
    (load_context(&name).id, name)
}

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    let day = resolve_date(args.fecha.as_deref(), Local::now().date_naive())?;
    let (cargue_id, _) = cargue_of(day);
    let mut cfg = RunConfig::new(&cargue_id);
    args.flags.apply(&mut cfg, args.workers)?;
    let events = Events::stdout(cfg.mode);
    let runner = PhaseRunner::real(&cfg);
    let download =
        |ctx: &AppContext, day: NaiveDate| convenio_descargar::download_range(ctx, day, day);
    let outcome = run_day(
        ctx,
        day,
        &cfg,
        args.redescargar,
        &download,
        &runner,
        &events,
    );
    // Stdout is the event stream in --json mode: the summary goes to stderr.
    let summary = outcome?;
    for line in summary.lines() {
        if cfg.mode == Mode::Json {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    if summary.failed() {
        bail!(
            "diario {day}: {} factura(s) fallaron; se reintentan en la próxima ejecución",
            summary.run.finished.failed
        );
    }
    Ok(())
}

/// The whole day with injectable download and phases.
pub fn run_day(
    ctx: &AppContext,
    day: NaiveDate,
    cfg: &RunConfig,
    redownload: bool,
    download: DownloadFn<'_>,
    runner: &PhaseRunner,
    events: &Events,
) -> Result<DiarioSummary> {
    let (cargue_id, name) = cargue_of(day);
    let csv = ensure_csv(ctx, cfg, day, &cargue_id, &name, redownload, download)?;
    let mut cfg = cfg.clone();
    cfg.source = cargue_id.clone();
    let run = execute(ctx, &cfg, runner, events)
        .with_context(|| format!("diario {day}: cargue {cargue_id}"))?;
    Ok(DiarioSummary {
        date: day,
        cargue_id,
        csv,
        run,
    })
}

/// True when the cargue is registered and its CSV artifact is published.
fn csv_available(ctx: &AppContext, cargue_id: &str, name: &str) -> Result<bool> {
    Ok(ctx.cargues().newest(cargue_id)?.is_some()
        && ctx.artifacts().is_published(cargue_id, name)?)
}

/// Reuses the registered CSV or downloads it; when another machine is
/// downloading the same day, waits for it and reuses what it published.
fn ensure_csv(
    ctx: &AppContext,
    cfg: &RunConfig,
    day: NaiveDate,
    cargue_id: &str,
    name: &str,
    redownload: bool,
    download: DownloadFn<'_>,
) -> Result<Csv> {
    let deadline = Instant::now() + cfg.tuning.prepare_wait;
    let lease = format!("convenio_descargar:{cargue_id}");
    let mut forced = redownload;
    loop {
        if !forced && csv_available(ctx, cargue_id, name)? {
            return Ok(Csv::Reused);
        }
        match download(ctx, day) {
            Ok(_) => return Ok(Csv::Downloaded),
            Err(err) => {
                let held = ctx.leases().holder(&lease).ok().flatten().is_some();
                if !held || Instant::now() >= deadline || crate::shutdown::requested() {
                    return Err(err.context(format!("diario {day}: descarga del convenio")));
                }
                // Someone else is downloading this day: wait, then reuse it.
                forced = false;
                std::thread::sleep(cfg.tuning.prepare_poll);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("date")
    }

    #[test]
    fn the_default_day_is_yesterday() {
        assert_eq!(
            resolve_date(None, day(2026, 10, 22)).expect("day"),
            day(2026, 10, 21)
        );
        assert_eq!(
            resolve_date(Some(" "), day(2026, 3, 1)).expect("day"),
            day(2026, 2, 28)
        );
        assert_eq!(
            resolve_date(None, day(2027, 1, 1)).expect("day"),
            day(2026, 12, 31)
        );
    }

    #[test]
    fn an_explicit_day_is_strict_and_not_in_the_future() {
        let today = day(2026, 10, 22);
        assert_eq!(
            resolve_date(Some("2026-10-05"), today).expect("day"),
            day(2026, 10, 5)
        );
        assert_eq!(
            resolve_date(Some("2026-10-22"), today).expect("today"),
            today
        );
        for bad in ["05/10/2026", "2026-1-5", "2026-02-30", "ayer"] {
            assert_eq!(
                resolve_date(Some(bad), today).expect_err(bad).to_string(),
                "--fecha must be YYYY-MM-DD"
            );
        }
        assert!(resolve_date(Some("2026-10-23"), today).is_err());
    }

    #[test]
    fn the_cargue_of_a_day_is_the_medicar_one() {
        let (id, name) = cargue_of(day(2026, 10, 21));
        assert_eq!(id, "Convenio_21102026_21102026");
        assert_eq!(name, "Convenio_21102026_21102026.csv");
    }
}
