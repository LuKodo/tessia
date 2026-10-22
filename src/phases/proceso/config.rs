//! The resolved settings of one `proceso run`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use super::events::Mode;
use super::pipeline::DEFAULT_BREAKER;
use super::steps::{Active, POOLED, default_workers};
use crate::domain::cargue::{is_valid_id, load_context};

/// Go default of `--batch-radicacion`.
pub const DEFAULT_BATCH: usize = 200;
/// Go default of `--radicacion-interval`.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(60);
/// Smallest number of candidates evaluated per replacement round.
pub const MIN_SUPPLY_CHUNK: usize = 5;

/// Knobs that tests shorten; the defaults are the production values.
#[derive(Debug, Clone)]
pub struct Tuning {
    /// Consecutive drops that pause replacement.
    pub breaker: usize,
    pub min_supply_chunk: usize,
    /// Pause between attempts to file a batch while another machine files the
    /// same cargue.
    pub radicacion_retry: Duration,
    /// Longest a batch waits for another machine's filing before it is left
    /// for the next run.
    pub radicacion_wait: Duration,
    /// Longest `proceso run` waits for another machine to finish preparing the
    /// same cargue.
    pub prepare_wait: Duration,
    pub prepare_poll: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            breaker: DEFAULT_BREAKER,
            min_supply_chunk: MIN_SUPPLY_CHUNK,
            radicacion_retry: Duration::from_secs(30),
            radicacion_wait: Duration::from_secs(30 * 60),
            prepare_wait: Duration::from_secs(30 * 60),
            prepare_poll: Duration::from_secs(2),
        }
    }
}

/// Everything `proceso run` needs, validated.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// A local CSV path or the id of an existing cargue (as in `convenio preparar`).
    pub source: String,
    /// `--invoices`, normalized: restrict the whole run to these.
    pub invoices: Vec<String>,
    pub limit: usize,
    pub no_replace: bool,
    /// Prepare again even when the cargue is already prepared.
    pub reprepare: bool,
    pub max_amount: Option<String>,
    pub cod_prestador: Option<String>,
    pub active: Active,
    pub continue_on_error: bool,
    pub batch_radicacion: usize,
    pub radicacion_interval: Duration,
    pub workers: BTreeMap<&'static str, usize>,
    pub mode: Mode,
    pub tuning: Tuning,
}

impl RunConfig {
    /// Defaults for `source`: every step, Go's pools, text output.
    pub fn new(source: &str) -> Self {
        Self {
            source: source.to_owned(),
            invoices: Vec::new(),
            limit: 0,
            no_replace: false,
            reprepare: false,
            max_amount: None,
            cod_prestador: None,
            active: Active::all(),
            continue_on_error: false,
            batch_radicacion: DEFAULT_BATCH,
            radicacion_interval: DEFAULT_INTERVAL,
            workers: POOLED.iter().map(|s| (*s, default_workers(s))).collect(),
            mode: Mode::Text,
            tuning: Tuning::default(),
        }
    }

    /// The pool size of a phase.
    pub fn workers_of(&self, step: &str) -> usize {
        self.workers
            .get(step)
            .copied()
            .unwrap_or_else(|| default_workers(step))
            .max(1)
    }

    /// The cargue the run processes: the stem of a CSV path, or the id.
    pub fn cargue_id(&self) -> Result<String> {
        let source = self.source.trim();
        if Path::new(source).is_file() {
            return Ok(load_context(source).id);
        }
        if is_valid_id(source) {
            return Ok(source.to_owned());
        }
        Err(anyhow!(
            "no existe el archivo {source:?} ni es un id de cargue válido"
        ))
    }
}

/// Parses a Go duration (`1m`, `30s`, `1h30m`, `250ms`, `0`).
pub fn parse_duration(flag: &str, text: &str) -> Result<Duration> {
    let text = text.trim();
    if text == "0" {
        return Ok(Duration::ZERO);
    }
    if text.starts_with('-') {
        bail!("{flag} must be >= 0");
    }
    if text.is_empty() {
        bail!("{flag}: invalid duration \"\"");
    }
    let mut total = Duration::ZERO;
    let mut rest = text.strip_prefix('+').unwrap_or(text);
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let (number, tail) = rest.split_at(digits);
        let unit_len = tail
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(tail.len());
        let (unit, next) = tail.split_at(unit_len);
        let value: f64 = number
            .parse()
            .map_err(|_| anyhow!("{flag}: invalid duration {text:?}"))?;
        let seconds = match unit {
            "ns" => 1e-9,
            "us" | "µs" => 1e-6,
            "ms" => 1e-3,
            "s" => 1.0,
            "m" => 60.0,
            "h" => 3600.0,
            _ => bail!("{flag}: invalid duration {text:?} (unit {unit:?})"),
        };
        total += Duration::from_secs_f64(value * seconds);
        rest = next;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_durations_parse() {
        let d = |t: &str| parse_duration("--x", t).expect(t);
        assert_eq!(d("1m"), Duration::from_secs(60));
        assert_eq!(d("30s"), Duration::from_secs(30));
        assert_eq!(d("1h30m"), Duration::from_secs(5400));
        assert_eq!(d("250ms"), Duration::from_millis(250));
        assert_eq!(d("0"), Duration::ZERO);
        assert_eq!(d("1.5s"), Duration::from_millis(1500));
        assert!(
            parse_duration("--x", "-5s")
                .expect_err("neg")
                .to_string()
                .contains(">= 0")
        );
        assert!(parse_duration("--x", "5").is_err());
        assert!(parse_duration("--x", "abc").is_err());
        assert!(parse_duration("--x", "").is_err());
    }

    #[test]
    fn the_cargue_id_is_the_csv_stem_or_the_id() {
        let dir = tempfile::tempdir().expect("dir");
        let csv = dir.path().join("Convenio_21102026_21102026.csv");
        std::fs::write(&csv, "x").expect("csv");
        let by_file = RunConfig::new(&csv.to_string_lossy());
        assert_eq!(
            by_file.cargue_id().expect("id"),
            "Convenio_21102026_21102026"
        );
        assert_eq!(RunConfig::new("C1").cargue_id().expect("id"), "C1");
        assert!(RunConfig::new("no existe/ni es id").cargue_id().is_err());
    }

    #[test]
    fn defaults_follow_go() {
        let cfg = RunConfig::new("C1");
        assert_eq!(cfg.batch_radicacion, 200);
        assert_eq!(cfg.radicacion_interval, Duration::from_secs(60));
        assert_eq!(cfg.workers_of("cuv"), 40);
        assert_eq!(cfg.workers_of("soporte"), 20);
    }
}
