//! Purely observational timings of a radicación run (Go
//! `internal/radicacion/timing`): where the wall-clock time goes (API calls,
//! limiter, database, local files, scheduled waits, queues). It never changes
//! what is sent, retried or waited: it only records counts and durations (no
//! invoice numbers, file names, ids or personal data).
//!
//! Memory is bounded: count, sum and max of every series are exact, while the
//! samples used for percentiles are kept up to [`SAMPLE_CAP`] per series
//! (uniform reservoir sampling beyond that, with a fixed seed). Percentiles
//! use the nearest-rank method.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Samples kept per series for percentiles.
pub const SAMPLE_CAP: usize = 10_000;

/// Scheduled-wait kinds: the delays the engine decides on.
pub mod sleep {
    pub const FIRST_POLL: &str = "primer_sondeo";
    pub const POLL: &str = "sondeo";
    pub const RESEND: &str = "reenvio";
    pub const SEND_RETRY: &str = "reintento_envio";
    pub const RELOAD: &str = "recarga";
    pub const BATCH_WAIT: &str = "espera_de_lote";
    pub const RESUME_DELAY: &str = "reanudacion";
}

/// Queue kinds: time spent waiting for a free worker.
pub mod queue {
    pub const SEND: &str = "envio";
    pub const CONFIRM: &str = "confirmacion";
}

/// Reads the (monotonic) time; replaceable in tests.
pub type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

#[derive(Debug, Default, Clone)]
struct Series {
    count: u64,
    sum: Duration,
    max: Duration,
    samples: Vec<Duration>,
    rng: u64,
}

impl Series {
    fn add(&mut self, d: Duration) {
        self.count += 1;
        self.sum += d;
        self.max = self.max.max(d);
        if self.samples.len() < SAMPLE_CAP {
            self.samples.push(d);
            return;
        }
        if self.rng == 0 {
            self.rng = 0x9E37_79B9_7F4A_7C15;
        }
        // xorshift64: a fixed seed keeps the sampling reproducible.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        let slot = (self.rng % self.count) as usize;
        if slot < SAMPLE_CAP {
            self.samples[slot] = d;
        }
    }

    /// Nearest-rank percentile `p` (0 < p <= 1).
    fn percentile(&self, p: f64) -> Duration {
        if self.samples.is_empty() {
            return Duration::ZERO;
        }
        let mut sorted = self.samples.clone();
        sorted.sort();
        let rank = (p * sorted.len() as f64).ceil() as usize;
        sorted[rank.clamp(1, sorted.len()) - 1]
    }
}

#[derive(Debug, Default)]
struct OpStat {
    lat: Series,
    errors: u64,
    retried: u64,
    limiter_wait: Duration,
    retry_wait: Duration,
}

#[derive(Debug, Default)]
struct StepStat {
    batches: u64,
    files: u64,
    prepare: Duration,
    send: Duration,
    latency: Series,
    polls: u64,
    polls_max: u64,
    confirmed: u64,
    resends: BTreeMap<String, u64>,
}

#[derive(Debug, Default, Clone, Copy)]
struct SleepStat {
    count: u64,
    total: Duration,
}

#[derive(Default)]
struct Inner {
    ops: BTreeMap<String, OpStat>,
    steps: [Option<StepStat>; 4],
    limiter: Series,
    conc_min: usize,
    conc_max: usize,
    conc_n: u64,
    db: Series,
    fs: Series,
    sleeps: BTreeMap<String, SleepStat>,
    queues: BTreeMap<String, SleepStat>,
}

/// The collector. Cheap to share (`Arc<Timings>`); every method takes `&self`.
pub struct Timings {
    clock: Clock,
    start: Instant,
    inner: Mutex<Inner>,
}

impl Timings {
    pub fn new() -> Timings {
        Timings::with_clock(Arc::new(Instant::now))
    }

    /// A collector reading time from `clock`; its wall time starts counting now.
    pub fn with_clock(clock: Clock) -> Timings {
        let start = clock();
        Timings {
            clock,
            start,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn now(&self) -> Instant {
        (self.clock)()
    }

    pub fn since(&self, t: Instant) -> Duration {
        self.now().saturating_duration_since(t)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// One API call of `op`: HTTP time (after the limiter granted a slot,
    /// retry backoff included), final outcome, and whether it retried.
    pub fn observe(&self, op: &str, d: Duration, failed: bool, retried: bool) {
        let mut inner = self.lock();
        let stat = inner.ops.entry(op.to_owned()).or_default();
        stat.lat.add(d);
        stat.errors += u64::from(failed);
        stat.retried += u64::from(retried);
    }

    /// Time a call of `op` waited for a limiter slot.
    pub fn observe_limiter_wait(&self, op: &str, d: Duration) {
        let mut inner = self.lock();
        inner.ops.entry(op.to_owned()).or_default().limiter_wait += d;
        inner.limiter.add(d);
    }

    /// Backoff slept inside the retries of `op` (already part of its HTTP time).
    pub fn observe_retry_wait(&self, op: &str, d: Duration) {
        self.lock().ops.entry(op.to_owned()).or_default().retry_wait += d;
    }

    /// The limiter concurrency limit seen when a slot was granted.
    pub fn observe_concurrency(&self, current: usize) {
        let mut inner = self.lock();
        if inner.conc_n == 0 || current < inner.conc_min {
            inner.conc_min = current;
        }
        if inner.conc_n == 0 || current > inner.conc_max {
            inner.conc_max = current;
        }
        inner.conc_n += 1;
    }

    /// One database statement (or group of them).
    pub fn observe_db(&self, d: Duration) {
        self.lock().db.add(d);
    }

    /// One local-file operation (find, read ZIP, archive, fetch).
    pub fn observe_fs(&self, d: Duration) {
        self.lock().fs.add(d);
    }

    /// A scheduled wait of `d` of the given kind.
    pub fn observe_sleep(&self, kind: &str, d: Duration) {
        if d.is_zero() {
            return;
        }
        let mut inner = self.lock();
        let stat = inner.sleeps.entry(kind.to_owned()).or_default();
        stat.count += 1;
        stat.total += d;
    }

    /// Time an item or batch waited for a free worker (zero waits are not
    /// recorded).
    pub fn observe_queue(&self, kind: &str, d: Duration) {
        if d.is_zero() {
            return;
        }
        let mut inner = self.lock();
        let stat = inner.queues.entry(kind.to_owned()).or_default();
        stat.count += 1;
        stat.total += d;
    }

    fn with_step(&self, step: usize, f: impl FnOnce(&mut StepStat)) {
        if !(1..=4).contains(&step) {
            return;
        }
        let mut inner = self.lock();
        f(inner.steps[step - 1].get_or_insert_with(StepStat::default));
    }

    /// One micro-batch of a step: its files, the time in `prepare` and the
    /// wall time of the upload call(s). A batch with no files adds its prepare
    /// time but does not count as a batch.
    pub fn step_batch(&self, step: usize, files: usize, prepare: Duration, send: Duration) {
        self.with_step(step, |s| {
            s.batches += u64::from(files > 0);
            s.files += files as u64;
            s.prepare += prepare;
            s.send += send;
        });
    }

    /// An invoice confirmed on `step`: time from its upload to the confirming
    /// verdict and how many polls that took (the confirming one included).
    pub fn confirmed(&self, step: usize, sent_to_confirmed: Duration, polls: u64) {
        self.with_step(step, |s| {
            s.confirmed += 1;
            s.latency.add(sent_to_confirmed);
            s.polls += polls;
            s.polls_max = s.polls_max.max(polls);
        });
    }

    /// An invoice of `step` was sent again, with a short reason.
    pub fn resend(&self, step: usize, reason: &str) {
        self.with_step(step, |s| *s.resends.entry(reason.to_owned()).or_default() += 1);
    }

    /// A render-ready copy of the figures so far.
    pub fn snapshot(&self) -> Snapshot {
        let wall = self.now().saturating_duration_since(self.start);
        let inner = self.lock();
        build_snapshot(&inner, wall)
    }
}

impl Default for Timings {
    fn default() -> Self {
        Timings::new()
    }
}

/// JSON schema version of `radicacion_tiempos.json`.
pub const SNAPSHOT_VERSION: u32 = 1;

/// A plain copy of what a [`Timings`] recorded; all durations in milliseconds.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub version: u32,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub wall_ms: f64,
    pub accum_ms: f64,
    pub steps: Vec<StepStats>,
    pub ops: Vec<OpStats>,
    pub limiter: LimiterStats,
    pub db: CallStats,
    pub files: CallStats,
    pub sleeps: Vec<SleepStats>,
    pub queues: Vec<SleepStats>,
    pub sinks: Vec<Sink>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepStats {
    pub step: usize,
    pub batches: u64,
    pub files: u64,
    pub prepare_ms: f64,
    pub send_ms: f64,
    pub confirmed: u64,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_max_ms: f64,
    pub polls_avg: f64,
    pub polls_max: u64,
    pub resends_total: u64,
    pub resends: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpStats {
    pub op: String,
    pub calls: u64,
    pub errors: u64,
    pub retried: u64,
    pub total_ms: f64,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub limiter_wait_ms: f64,
    pub retry_wait_ms: f64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LimiterStats {
    pub waits: u64,
    pub wait_ms: f64,
    pub max_wait_ms: f64,
    pub samples: u64,
    pub min_concurrency: usize,
    pub max_concurrency: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallStats {
    pub calls: u64,
    pub total_ms: f64,
    pub max_ms: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SleepStats {
    pub kind: String,
    pub count: u64,
    pub total_ms: f64,
}

/// One non-overlapping category of accumulated time.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sink {
    pub name: String,
    pub ms: f64,
    pub pct_accum: f64,
    pub pct_wall: f64,
}

fn round(value: f64, places: i32) -> f64 {
    let p = 10f64.powi(places);
    (value * p).round() / p
}

fn ms_of(d: Duration) -> f64 {
    round(d.as_secs_f64() * 1000.0, 3)
}

fn call_stats(s: &Series) -> CallStats {
    CallStats {
        calls: s.count,
        total_ms: ms_of(s.sum),
        max_ms: ms_of(s.max),
    }
}

fn step_stats(n: usize, s: &StepStat) -> StepStats {
    StepStats {
        step: n,
        batches: s.batches,
        files: s.files,
        prepare_ms: ms_of(s.prepare),
        send_ms: ms_of(s.send),
        confirmed: s.confirmed,
        latency_p50_ms: ms_of(s.latency.percentile(0.50)),
        latency_p95_ms: ms_of(s.latency.percentile(0.95)),
        latency_max_ms: ms_of(s.latency.max),
        polls_avg: if s.confirmed > 0 {
            round(s.polls as f64 / s.confirmed as f64, 3)
        } else {
            0.0
        },
        polls_max: s.polls_max,
        resends_total: s.resends.values().sum(),
        resends: s.resends.clone(),
    }
}

fn op_stats(name: &str, o: &OpStat) -> OpStats {
    let total_ms = ms_of(o.lat.sum);
    OpStats {
        op: name.to_owned(),
        calls: o.lat.count,
        errors: o.errors,
        retried: o.retried,
        total_ms,
        avg_ms: if o.lat.count > 0 {
            round(total_ms / o.lat.count as f64, 3)
        } else {
            0.0
        },
        p50_ms: ms_of(o.lat.percentile(0.50)),
        p95_ms: ms_of(o.lat.percentile(0.95)),
        max_ms: ms_of(o.lat.max),
        limiter_wait_ms: ms_of(o.limiter_wait),
        retry_wait_ms: ms_of(o.retry_wait),
    }
}

fn sleep_stats(map: &BTreeMap<String, SleepStat>) -> Vec<SleepStats> {
    map.iter()
        .map(|(kind, s)| SleepStats {
            kind: kind.clone(),
            count: s.count,
            total_ms: ms_of(s.total),
        })
        .collect()
}

fn build_snapshot(inner: &Inner, wall: Duration) -> Snapshot {
    let steps: Vec<StepStats> = inner
        .steps
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.as_ref().map(|s| step_stats(i + 1, s)))
        .collect();
    let ops: Vec<OpStats> = inner.ops.iter().map(|(n, o)| op_stats(n, o)).collect();
    let limiter = LimiterStats {
        waits: inner.limiter.count,
        wait_ms: ms_of(inner.limiter.sum),
        max_wait_ms: ms_of(inner.limiter.max),
        samples: inner.conc_n,
        min_concurrency: inner.conc_min,
        max_concurrency: inner.conc_max,
    };
    let (db, files) = (call_stats(&inner.db), call_stats(&inner.fs));
    let sleeps = sleep_stats(&inner.sleeps);
    let queues = sleep_stats(&inner.queues);
    let wall_ms = ms_of(wall);

    let mut raw: Vec<(String, f64)> = ops
        .iter()
        .filter(|o| o.total_ms > 0.0)
        .map(|o| (format!("API {}", o.op), o.total_ms))
        .collect();
    raw.extend(
        sleeps
            .iter()
            .map(|s| (format!("Espera programada: {}", s.kind), s.total_ms)),
    );
    raw.extend(
        queues
            .iter()
            .map(|q| (format!("Cola: {}", q.kind), q.total_ms)),
    );
    raw.push(("Espera en el limitador".to_owned(), limiter.wait_ms));
    raw.push(("Base de datos".to_owned(), db.total_ms));
    raw.push(("Archivos locales".to_owned(), files.total_ms));
    raw.retain(|(_, ms)| *ms > 0.0);
    let accum_ms = round(raw.iter().map(|(_, ms)| ms).sum(), 3);
    let mut sinks: Vec<Sink> = raw
        .into_iter()
        .map(|(name, ms)| Sink {
            name,
            ms,
            pct_accum: if accum_ms > 0.0 {
                round(ms / accum_ms * 100.0, 1)
            } else {
                0.0
            },
            pct_wall: if wall_ms > 0.0 {
                round(ms / wall_ms * 100.0, 1)
            } else {
                0.0
            },
        })
        .collect();
    sinks.sort_by(|a, b| {
        b.ms.partial_cmp(&a.ms)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });
    Snapshot {
        version: SNAPSHOT_VERSION,
        generated_at: chrono::Utc::now(),
        wall_ms,
        accum_ms,
        steps,
        ops,
        limiter,
        db,
        files,
        sleeps,
        queues,
        sinks,
    }
}

/// How many time sinks the text block lists.
const TOP_SINKS: usize = 8;

fn dur(ms: f64) -> String {
    if ms.abs() < 1000.0 {
        format!("{ms:.1} ms")
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

fn sec(ms: f64) -> String {
    format!("{:.1}", ms / 1000.0)
}

/// Go `time.Duration.String()` of whole seconds (`7m12s`, `1h0m0s`, `0s`).
fn whole_seconds(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    match (h, m) {
        (0, 0) => format!("{s}s"),
        (0, _) => format!("{m}m{s}s"),
        _ => format!("{h}h{m}m{s}s"),
    }
}

fn resends_text(map: &BTreeMap<String, u64>) -> String {
    if map.is_empty() {
        return "-".to_owned();
    }
    map.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Aligns tab-separated cells like Go's `tabwriter` (padding 2, spaces): every
/// cell but the last of a line is padded to the widest cell of its column.
fn table(rows: &[Vec<String>]) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
                + 2
        })
        .collect();
    let mut out = String::new();
    for row in rows {
        for (c, cell) in row.iter().enumerate() {
            out.push_str(cell);
            if c + 1 < row.len() {
                let pad = widths[c].saturating_sub(cell.chars().count());
                out.push_str(&" ".repeat(pad));
            }
        }
        out.push('\n');
    }
    out
}

impl Snapshot {
    fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.ops.is_empty() && self.sinks.is_empty()
    }

    /// The Spanish "Tiempos de radicación" block.
    pub fn text(&self) -> String {
        let mut b = String::from("Tiempos de radicación\n");
        let wall_secs = (self.wall_ms / 1000.0).round().max(0.0) as u64;
        b.push_str(&format!(
            "Duración total (reloj): {}\n",
            whole_seconds(wall_secs)
        ));
        if self.is_empty() {
            b.push_str("(sin actividad medida)\n");
            return b;
        }
        b.push_str("Nota: El trabajo corre en paralelo (varios lotes, envíos y confirmaciones a la vez): el tiempo acumulado\n");
        b.push_str("suma todas las llamadas, esperas y escrituras y puede superar el reloj; sobre el reloj los porcentajes pueden pasar de 100%.\n");
        if !self.steps.is_empty() {
            b.push_str("\nPor paso (latencia = desde el envío hasta la confirmación; no incluye lo ocurrido en corridas anteriores)\n");
            b.push_str(&self.steps_table());
        }
        if !self.ops.is_empty() {
            b.push_str("\nPor operación de API (tiempo HTTP, incluye la espera entre reintentos; el limitador va aparte)\n");
            b.push_str(&self.ops_table());
        }
        b.push('\n');
        b.push_str(&self.summary_lines());
        b
    }

    fn steps_table(&self) -> String {
        let header = "PASO\tLOTES\tARCHIVOS\tPREP(s)\tENVÍO(s)\tCONFIRMADAS\tENV→CONF p50(s)\tp95(s)\tmáx(s)\tSONDEOS/FACT\tREENVÍOS";
        let mut rows = vec![header.split('\t').map(str::to_owned).collect::<Vec<_>>()];
        for s in &self.steps {
            rows.push(vec![
                s.step.to_string(),
                s.batches.to_string(),
                s.files.to_string(),
                sec(s.prepare_ms),
                sec(s.send_ms),
                s.confirmed.to_string(),
                sec(s.latency_p50_ms),
                sec(s.latency_p95_ms),
                sec(s.latency_max_ms),
                format!("{:.1}", s.polls_avg),
                resends_text(&s.resends),
            ]);
        }
        table(&rows)
    }

    fn ops_table(&self) -> String {
        let header = "OPERACIÓN\tLLAMADAS\tERRORES\tREINTENTOS\tPROM(ms)\tP95(ms)\tMÁX(ms)\tESP.LIMITADOR(ms total)\tESP.REINTENTOS(ms total)";
        let mut rows = vec![header.split('\t').map(str::to_owned).collect::<Vec<_>>()];
        for o in &self.ops {
            rows.push(vec![
                o.op.clone(),
                o.calls.to_string(),
                o.errors.to_string(),
                o.retried.to_string(),
                format!("{:.1}", o.avg_ms),
                format!("{:.1}", o.p95_ms),
                format!("{:.1}", o.max_ms),
                format!("{:.1}", o.limiter_wait_ms),
                format!("{:.1}", o.retry_wait_ms),
            ]);
        }
        table(&rows)
    }

    fn summary_lines(&self) -> String {
        let mut b = String::new();
        let l = &self.limiter;
        if l.waits > 0 || l.samples > 0 {
            b.push_str(&format!(
                "Limitador: espera total {} en {} adquisiciones (máx {})",
                dur(l.wait_ms),
                l.waits,
                dur(l.max_wait_ms)
            ));
            if l.samples > 0 {
                b.push_str(&format!(
                    "; concurrencia observada {}-{}",
                    l.min_concurrency, l.max_concurrency
                ));
            }
            b.push('\n');
        }
        b.push_str(&format!(
            "Base de datos: {} llamadas, {} (máx {})\n",
            self.db.calls,
            dur(self.db.total_ms),
            dur(self.db.max_ms)
        ));
        b.push_str(&format!(
            "Archivos locales: {} llamadas, {} (máx {})\n",
            self.files.calls,
            dur(self.files.total_ms),
            dur(self.files.max_ms)
        ));
        if !self.sleeps.is_empty() {
            let parts: Vec<String> = self
                .sleeps
                .iter()
                .map(|s| format!("{}={}", s.kind, dur(s.total_ms)))
                .collect();
            b.push_str(&format!("Esperas programadas: {} (suma de las esperas decididas por el motor, por factura)\n", parts.join(", ")));
        }
        if !self.queues.is_empty() {
            let parts: Vec<String> = self
                .queues
                .iter()
                .map(|q| format!("{}={}", q.kind, dur(q.total_ms)))
                .collect();
            b.push_str(&format!(
                "Colas (espera por un trabajador libre): {}\n",
                parts.join(", ")
            ));
        }
        if !self.sinks.is_empty() {
            b.push_str(&format!(
                "\nTiempo acumulado: {} (reloj: {})\n",
                dur(self.accum_ms),
                dur(self.wall_ms)
            ));
            b.push_str("Principales consumos de tiempo (% del acumulado, % del reloj)\n");
            for (i, k) in self.sinks.iter().take(TOP_SINKS).enumerate() {
                b.push_str(&format!(
                    "  {}. {}  {}  ({:.1}% del acumulado, {:.1}% del reloj)\n",
                    i + 1,
                    k.name,
                    dur(k.ms),
                    k.pct_accum,
                    k.pct_wall
                ));
            }
        }
        b
    }

    /// The snapshot as indented JSON (counts and durations only).
    pub fn json(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// The summary as one structured line (`radicacion timings`).
    pub fn log(&self) {
        let steps: Vec<String> = self
            .steps
            .iter()
            .map(|s| {
                format!(
                    "step{}[batches={} files={} confirmed={} resends={}]",
                    s.step, s.batches, s.files, s.confirmed, s.resends_total
                )
            })
            .collect();
        let api: Vec<String> = self
            .ops
            .iter()
            .map(|o| format!("{}[calls={} errors={} avg_ms={}]", o.op, o.calls, o.errors, o.avg_ms))
            .collect();
        tracing::info!(
            wall_ms = self.wall_ms,
            accum_ms = self.accum_ms,
            steps = %steps.join(" "),
            api = %api.join(" "),
            limiter_wait_ms = self.limiter.wait_ms,
            db_calls = self.db.calls,
            db_total_ms = self.db.total_ms,
            files_calls = self.files.calls,
            files_total_ms = self.files.total_ms,
            top_sink = self.sinks.first().map(|s| s.name.as_str()).unwrap_or(""),
            "radicacion timings"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clock the test moves by hand.
    fn manual_clock() -> (Clock, Arc<Mutex<Instant>>) {
        let now = Arc::new(Mutex::new(Instant::now()));
        let shared = now.clone();
        let clock: Clock = Arc::new(move || {
            *shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        });
        (clock, now)
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let mut s = Series::default();
        for n in [10, 20, 30, 40, 50, 60, 70, 80, 90, 100] {
            s.add(ms(n));
        }
        assert_eq!(s.percentile(0.50), ms(50));
        assert_eq!(s.percentile(0.95), ms(100));
        assert_eq!(s.percentile(0.10), ms(10));
        assert_eq!(Series::default().percentile(0.5), Duration::ZERO);
    }

    #[test]
    fn reservoir_keeps_exact_count_sum_and_max() {
        let mut s = Series::default();
        for n in 0..(SAMPLE_CAP as u64 + 500) {
            s.add(ms(n % 7 + 1));
        }
        assert_eq!(s.count, SAMPLE_CAP as u64 + 500);
        assert_eq!(s.samples.len(), SAMPLE_CAP);
        assert_eq!(s.max, ms(7));
    }

    #[test]
    fn snapshot_aggregates_and_ranks_the_sinks() {
        let (clock, now) = manual_clock();
        let tm = Timings::with_clock(clock);
        tm.observe_limiter_wait("find-load", ms(30));
        tm.observe_concurrency(20);
        tm.observe_concurrency(23);
        tm.observe("find-load", ms(350), false, false);
        tm.observe("find-load", ms(450), true, true);
        tm.observe_retry_wait("find-load", ms(100));
        tm.observe_sleep(sleep::POLL, ms(600));
        tm.observe_sleep(sleep::POLL, Duration::ZERO);
        tm.observe_queue(queue::CONFIRM, ms(135));
        tm.observe_db(ms(15));
        tm.step_batch(1, 200, ms(4000), ms(95000));
        tm.step_batch(1, 0, ms(1000), Duration::ZERO);
        tm.confirmed(1, ms(40000), 4);
        tm.confirmed(1, ms(60000), 3);
        tm.resend(1, "quick_retry");
        tm.resend(1, "send_error");
        tm.resend(9, "ignored");
        *now.lock().unwrap() += Duration::from_secs(432);
        let snap = tm.snapshot();

        assert_eq!(snap.wall_ms, 432_000.0);
        let step = &snap.steps[0];
        assert_eq!((step.step, step.batches, step.files), (1, 1, 200));
        assert_eq!((step.prepare_ms, step.send_ms), (5000.0, 95000.0));
        assert_eq!((step.confirmed, step.polls_avg, step.polls_max), (2, 3.5, 4));
        assert_eq!((step.latency_p50_ms, step.latency_max_ms), (40000.0, 60000.0));
        assert_eq!(step.resends_total, 2);
        let op = &snap.ops[0];
        assert_eq!((op.calls, op.errors, op.retried), (2, 1, 1));
        assert_eq!((op.avg_ms, op.limiter_wait_ms, op.retry_wait_ms), (400.0, 30.0, 100.0));
        assert_eq!((snap.limiter.min_concurrency, snap.limiter.max_concurrency), (20, 23));
        assert_eq!(snap.sleeps.len(), 1);
        assert_eq!(snap.sleeps[0].count, 1);
        let names: Vec<&str> = snap.sinks.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "API find-load",
                "Espera programada: sondeo",
                "Cola: confirmacion",
                "Espera en el limitador",
                "Base de datos"
            ]
        );
        assert_eq!(snap.accum_ms, 1580.0);
    }

    #[test]
    fn text_block_is_the_spanish_report() {
        let (clock, now) = manual_clock();
        let tm = Timings::with_clock(clock);
        tm.observe("upload", ms(1200), false, false);
        tm.step_batch(2, 3, ms(2000), ms(40000));
        tm.resend(2, "quick_retry");
        *now.lock().unwrap() += Duration::from_secs(432);
        let text = tm.snapshot().text();
        assert!(text.starts_with("Tiempos de radicación\nDuración total (reloj): 7m12s\nNota: "), "{text}");
        assert!(text.contains("Por paso (latencia = desde el envío hasta la confirmación"));
        assert!(text.contains("PASO  LOTES  ARCHIVOS  PREP(s)  ENVÍO(s)  CONFIRMADAS  ENV→CONF p50(s)  p95(s)  máx(s)  SONDEOS/FACT  REENVÍOS"));
        assert!(text.contains("2     1      3         2.0      40.0      0            0.0              0.0     0.0     0.0           quick_retry=1"), "{text}");
        assert!(text.contains("upload     1         0        0           1200.0    1200.0   1200.0   0.0                      0.0"), "{text}");
        assert!(text.contains("Base de datos: 0 llamadas, 0.0 ms (máx 0.0 ms)"));
        assert!(text.contains("Tiempo acumulado: 1.2 s (reloj: 432.0 s)"));
        assert!(text.contains("  1. API upload  1.2 s  (100.0% del acumulado, 0.3% del reloj)"));
    }

    #[test]
    fn empty_snapshot_says_so_and_json_is_camel_case() {
        let (clock, _) = manual_clock();
        let tm = Timings::with_clock(clock);
        let snap = tm.snapshot();
        assert_eq!(
            snap.text(),
            "Tiempos de radicación\nDuración total (reloj): 0s\n(sin actividad medida)\n"
        );
        let json: serde_json::Value = serde_json::from_slice(&snap.json().unwrap()).unwrap();
        for key in ["version", "generatedAt", "wallMs", "accumMs", "steps", "ops", "limiter", "db", "files", "sleeps", "queues", "sinks"] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
        assert_eq!(json["version"], 1);
    }

    #[test]
    fn go_style_durations() {
        assert_eq!(whole_seconds(0), "0s");
        assert_eq!(whole_seconds(45), "45s");
        assert_eq!(whole_seconds(432), "7m12s");
        assert_eq!(whole_seconds(3600), "1h0m0s");
    }
}
