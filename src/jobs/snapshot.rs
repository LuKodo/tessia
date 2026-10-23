//! A job's live state and the snapshot the dashboard polls (Go `job`,
//! `jobSnapshot`, `jobStats`, `jobConvenio`, `jobPhaseProgress`).
//!
//! [`JobState::observe`] feeds the tallies from the `proceso run` events; the
//! snapshot is plain serializable data with Go's omission rules.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, SecondsFormat};
use serde::{Deserialize, Serialize};

use super::synthetic_log::{self, JobError, SyntheticInput};
use crate::domain::radicacion::Step;
use crate::phases::proceso::Event;

/// Go's zero `time.Time` as JSON.
pub const ZERO_TIME: &str = "0001-01-01T00:00:00Z";

/// Distinct errors the synthetic log lists; the rest are only counted.
pub const MAX_JOB_ERRORS: usize = 5;
/// Bound on the memory spent telling distinct errors apart.
pub const MAX_JOB_ERROR_KEYS: usize = 4096;

/// The phase key of the radicación meter (a batch step, not a per-invoice one).
const RADICACION: &str = "radicacion";
const STATUS_OK: &str = "OK";
const STATUS_MANUAL: &str = "MANUAL_REVIEW";
const STATUS_EXCLUDED: &str = "EXCLUDED";
const STATUS_FAILED: &str = "FAILED";

/// States of [`JobConvenio::estado`].
pub const CONVENIO_PREPARING: &str = "preparando";
pub const CONVENIO_OK: &str = "ok";
pub const CONVENIO_ERROR: &str = "error";

/// RFC 3339 like Go's `time.Time` JSON (local offset, `Z` for UTC).
pub fn rfc3339(time: DateTime<Local>) -> String {
    time.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// The per-run invoice tally shown next to a finished `proceso run`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct JobStats {
    pub esperadas: usize,
    pub procesadas: usize,
    pub reemplazadas: usize,
    pub gestionadas: usize,
    pub terminadas: usize,
    pub con_errores: usize,
    pub excluidas: usize,
    pub revision_manual: usize,
    pub evaluadas: usize,
    pub limite: usize,
}

/// One phase's live tally (Go `jobPhaseProgress`).
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct PhaseProgress {
    pub fase: String,
    pub etiqueta: String,
    pub en_curso: usize,
    pub terminadas: usize,
    pub revision_manual: usize,
    pub excluidas: usize,
    pub con_errores: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub tiempo_ms: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Step 0 (`convenio preparar`) as the dashboard sees it.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct JobConvenio {
    pub estado: String,
    pub evaluadas: usize,
    pub total: usize,
    pub preparadas: usize,
    pub excluidas: usize,
    pub revision_manual: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mensaje: String,
}

/// A job as `GET /api/jobs/current` and the job-starting routes answer it.
/// `terminado_en` is always present (the zero time while running); the other
/// optional fields are omitted when empty.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct JobSnapshot {
    pub id: String,
    pub tipo: String,
    pub corriendo: bool,
    pub cancelable: bool,
    pub iniciado_en: String,
    pub terminado_en: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    pub salida: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estadisticas: Option<JobStats>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fases: Vec<PhaseProgress>,
    #[serde(skip_serializing_if = "is_zero_usize")]
    pub radicadas: usize,
    /// Radicación split into its four steps; empty until the first batch starts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub radicacion_pasos: Vec<PhaseProgress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub convenio: Option<JobConvenio>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instancia_servidor: String,
}

fn is_zero_usize(n: &usize) -> bool {
    *n == 0
}

impl JobSnapshot {
    /// What the dashboard gets before any job ran in this process.
    pub fn idle(instance: &str) -> Self {
        Self {
            iniciado_en: ZERO_TIME.to_owned(),
            terminado_en: ZERO_TIME.to_owned(),
            instancia_servidor: instance.to_owned(),
            ..Self::default()
        }
    }
}

/// The monotonic clock busy times are measured with (tests inject a fake).
pub type Clock = std::sync::Arc<dyn Fn() -> Instant + Send + Sync>;

/// The real clock.
pub fn system_clock() -> Clock {
    std::sync::Arc::new(Instant::now)
}

/// A phase tally plus its busy-time accounting.
struct Tally {
    progress: PhaseProgress,
    busy: Duration,
    busy_since: Option<Instant>,
}

impl Tally {
    fn new(key: &str, label: &str) -> Self {
        Self {
            progress: PhaseProgress {
                fase: key.to_owned(),
                etiqueta: label.to_owned(),
                ..PhaseProgress::default()
            },
            busy: Duration::ZERO,
            busy_since: None,
        }
    }

    fn mark_busy(&mut self, now: Instant) {
        self.busy_since.get_or_insert(now);
    }

    fn mark_idle(&mut self, now: Instant) {
        if let Some(since) = self.busy_since.take() {
            self.busy += now.saturating_duration_since(since);
        }
    }

    fn busy_at(&self, now: Instant) -> Duration {
        match self.busy_since {
            Some(since) => self.busy + now.saturating_duration_since(since),
            None => self.busy,
        }
    }

    /// No event ever reached the phase (only a seeded entry can be in this state).
    fn untouched(&self) -> bool {
        let p = &self.progress;
        p.en_curso == 0
            && p.terminadas == 0
            && p.revision_manual == 0
            && p.excluidas == 0
            && p.con_errores == 0
            && self.busy.is_zero()
            && self.busy_since.is_none()
    }
}

/// Everything the dashboard shows about one job. Wrapped in a mutex by the
/// runner: the pipeline workers feed it concurrently.
pub struct JobState {
    pub id: String,
    pub kind: String,
    pub started_at: DateTime<Local>,
    instance: String,
    clock: Clock,
    pub running: bool,
    pub cancelled: bool,
    pub ended_at: Option<DateTime<Local>>,
    pub error: String,
    stats: Option<JobStats>,
    /// Convenio-stage exclusions and manual reviews, kept apart so `run_end`
    /// can add the pipeline's own on top.
    prep_excluded: usize,
    prep_manual: usize,
    convenio: Option<JobConvenio>,
    phases: Vec<Tally>,
    /// The four radicación steps, created together by the first step event.
    radicacion_steps: Vec<PhaseProgress>,
    radicacion_done: usize,
    cargue: String,
    plan: Vec<String>,
    run_ended: bool,
    errors: Vec<JobError>,
    error_keys: HashSet<String>,
    error_total: usize,
}

impl JobState {
    pub fn new(id: &str, kind: &str, instance: &str, clock: Clock) -> Self {
        Self {
            id: id.to_owned(),
            kind: kind.to_owned(),
            started_at: Local::now(),
            instance: instance.to_owned(),
            clock,
            running: true,
            cancelled: false,
            ended_at: None,
            error: String::new(),
            stats: None,
            prep_excluded: 0,
            prep_manual: 0,
            convenio: None,
            phases: Vec::new(),
            radicacion_steps: Vec::new(),
            radicacion_done: 0,
            cargue: String::new(),
            plan: Vec::new(),
            run_ended: false,
            errors: Vec::new(),
            error_keys: HashSet::new(),
            error_total: 0,
        }
    }

    /// Which cargue and phases the run covers, for the synthetic start line.
    pub fn describe(&mut self, cargue: &str, plan: Vec<String>) {
        self.cargue = cargue.to_owned();
        self.plan = plan;
    }

    fn phase(&mut self, key: &str, label: &str) -> &mut Tally {
        let at = match self.phases.iter().position(|t| t.progress.fase == key) {
            Some(at) => at,
            None => {
                self.phases.push(Tally::new(key, label));
                self.phases.len() - 1
            }
        };
        &mut self.phases[at]
    }

    /// Lists the planned phases from the start, all queued (zero counters), in
    /// plan order. Later events fill the same entries in place; an entry an
    /// early event already created keeps its tally and only moves to its plan
    /// position.
    pub fn seed_phases(&mut self, plan: &[(String, String)]) {
        let mut ordered: Vec<Tally> = Vec::with_capacity(plan.len() + self.phases.len());
        for (key, label) in plan {
            if ordered.iter().any(|t| &t.progress.fase == key) {
                continue;
            }
            let at = self.phases.iter().position(|t| &t.progress.fase == key);
            ordered.push(match at {
                Some(at) => self.phases.remove(at),
                None => Tally::new(key, label),
            });
        }
        ordered.append(&mut self.phases);
        self.phases = ordered;
    }

    /// Stops every open busy interval so a finished or cancelled job's times freeze.
    pub fn close_busy(&mut self) {
        let now = (self.clock)();
        for tally in &mut self.phases {
            tally.mark_idle(now);
        }
    }

    fn stats_mut(&mut self) -> &mut JobStats {
        self.stats.get_or_insert_with(JobStats::default)
    }

    fn observe_convenio(&mut self, e: &Event) {
        match e.kind.as_str() {
            "convenio_start" => {
                self.convenio = Some(JobConvenio {
                    estado: CONVENIO_PREPARING.to_owned(),
                    ..JobConvenio::default()
                });
            }
            "convenio_progress" => {
                let convenio = self.convenio.get_or_insert_with(|| JobConvenio {
                    estado: CONVENIO_PREPARING.to_owned(),
                    ..JobConvenio::default()
                });
                if convenio.estado != CONVENIO_PREPARING {
                    return; // a late progress event never reopens a finished step
                }
                convenio.evaluadas = e.evaluated;
                convenio.total = e.total;
            }
            "convenio" => {
                let convenio = self.convenio.get_or_insert_with(JobConvenio::default);
                if e.status != STATUS_OK {
                    convenio.estado = CONVENIO_ERROR.to_owned();
                    convenio.mensaje = e.error.clone();
                    return;
                }
                convenio.estado = CONVENIO_OK.to_owned();
                convenio.preparadas = e.done;
                convenio.excluidas = e.excluded;
                convenio.revision_manual = e.manual;
                if convenio.total == 0 {
                    convenio.evaluadas = e.evaluated;
                }
            }
            _ => {}
        }
    }

    /// Tallies one `proceso run` event (Go `job.observe`). A run that never
    /// reaches `run_end` keeps only what it saw; a failed `convenio` event's
    /// counts are not a result.
    pub fn observe(&mut self, e: &Event) {
        match e.kind.as_str() {
            "convenio_start" | "convenio_progress" => self.observe_convenio(e),
            "convenio" => {
                self.observe_convenio(e);
                if e.status != STATUS_OK {
                    return;
                }
                self.prep_excluded = e.excluded;
                self.prep_manual = e.manual;
                let stats = self.stats_mut();
                stats.evaluadas = e.evaluated;
                stats.limite = e.limit;
                stats.excluidas = e.excluded;
                stats.revision_manual = e.manual;
            }
            "convenio_more" => {
                // An extra preparation round: its counts are deltas.
                self.prep_excluded += e.excluded;
                self.prep_manual += e.manual;
                let stats = self.stats_mut();
                stats.evaluadas += e.evaluated;
                stats.excluidas += e.excluded;
                stats.revision_manual += e.manual;
            }
            "invoice_replace" => {
                let stats = self.stats_mut();
                stats.reemplazadas += 1;
                stats.procesadas += 1;
            }
            "run_start" => {
                let (excluded, manual) = (self.prep_excluded, self.prep_manual);
                let stats = self.stats_mut();
                stats.esperadas = e.total;
                stats.procesadas = e.total;
                stats.reemplazadas = 0;
                stats.gestionadas = 0;
                stats.terminadas = 0;
                stats.con_errores = 0;
                stats.excluidas = excluded;
                stats.revision_manual = manual;
            }
            "invoice_enter" => {
                let now = (self.clock)();
                let tally = self.phase(&e.phase, &e.step);
                tally.progress.en_curso += 1;
                tally.mark_busy(now);
            }
            "invoice_step" => self.observe_step(e),
            "radicacion_enter" => {
                self.phase(RADICACION, &e.step).progress.en_curso += 1;
            }
            "radicacion" => self.observe_radicacion(e),
            "radicacion_step" => self.observe_radicacion_step(e),
            "run_end" => self.observe_run_end(e),
            _ => {}
        }
    }

    fn observe_step(&mut self, e: &Event) {
        let now = (self.clock)();
        let tally = self.phase(&e.phase, &e.step);
        tally.progress.en_curso = tally.progress.en_curso.saturating_sub(1);
        if tally.progress.en_curso == 0 {
            tally.mark_idle(now);
        }
        match e.status.as_str() {
            STATUS_OK => tally.progress.terminadas += 1,
            STATUS_MANUAL => tally.progress.revision_manual += 1,
            STATUS_EXCLUDED => tally.progress.excluidas += 1,
            STATUS_FAILED => {
                tally.progress.con_errores += 1;
                self.record_error(&e.invoice, &e.step, error_detail(e));
            }
            _ => {} // SKIPPED: another machine owns the invoice, nothing to count
        }
    }

    /// One invoice moving through a radicación step (`e.phase` = `radicacion_N`).
    fn observe_radicacion_step(&mut self, e: &Event) {
        if self.radicacion_steps.is_empty() {
            self.radicacion_steps = Step::ALL
                .iter()
                .map(|step| PhaseProgress {
                    fase: format!("radicacion_{}", step.as_i16()),
                    etiqueta: format!("Radicación · paso {}: {}", step.as_i16(), step.label()),
                    ..PhaseProgress::default()
                })
                .collect();
        }
        let Some(progress) = self.radicacion_steps.iter_mut().find(|p| p.fase == e.phase) else {
            return;
        };
        match e.status.as_str() {
            "ENTER" => progress.en_curso += 1,
            "DONE" => {
                progress.en_curso = progress.en_curso.saturating_sub(1);
                progress.terminadas += 1;
            }
            "BACK" => progress.en_curso = progress.en_curso.saturating_sub(1),
            "FAILED" => {
                progress.en_curso = progress.en_curso.saturating_sub(1);
                progress.con_errores += 1;
            }
            _ => {}
        }
    }

    fn observe_radicacion(&mut self, e: &Event) {
        let tally = self.phase(RADICACION, &e.step);
        tally.progress.en_curso = tally.progress.en_curso.saturating_sub(e.batch);
        // Radicación time is the batches' own send time, not the queue wait.
        tally.busy += Duration::from_millis(e.duration_ms);
        match e.status.as_str() {
            STATUS_OK => {
                tally.progress.terminadas += e.batch;
                self.radicacion_done += e.batch;
            }
            STATUS_FAILED => {
                tally.progress.con_errores += e.batch;
                let who = match e.invoices.as_slice() {
                    [one] => one.clone(),
                    _ => format!("lote de {} facturas", e.batch),
                };
                self.record_error(&who, &e.step, error_detail(e));
            }
            _ => {} // SKIPPED: left queued for the next run
        }
    }

    fn observe_run_end(&mut self, e: &Event) {
        self.run_ended = true;
        let (excluded, manual) = (self.prep_excluded, self.prep_manual);
        let Some(stats) = self.stats.as_mut() else {
            return;
        };
        stats.terminadas = e.done;
        stats.con_errores = e.failed;
        stats.excluidas = excluded + e.excluded;
        stats.revision_manual = manual + e.manual;
        stats.gestionadas = e.done + e.failed + e.excluded + e.manual;
        // `run_end.total` counts every invoice that entered the pipeline, but
        // only when replacement was active; otherwise nothing was added.
        stats.procesadas = if e.total > 0 {
            e.total
        } else {
            stats.esperadas
        };
        stats.reemplazadas = e.replaced;
    }

    /// Registers one failure for the synthetic log: the first
    /// [`MAX_JOB_ERRORS`] distinct ones are kept, later distinct ones only
    /// counted. Distinct means invoice + phase + reason.
    fn record_error(&mut self, invoice: &str, phase: &str, reason: &str) {
        let key = format!("{invoice}\0{phase}\0{reason}");
        if self.error_keys.contains(&key) {
            return;
        }
        if self.error_keys.len() < MAX_JOB_ERROR_KEYS {
            self.error_keys.insert(key);
        }
        self.error_total += 1;
        if self.errors.len() < MAX_JOB_ERRORS {
            let reason = if reason.trim().is_empty() {
                "sin detalle"
            } else {
                reason
            };
            self.errors.push(JobError {
                invoice: invoice.to_owned(),
                phase: phase.to_owned(),
                reason: reason.to_owned(),
            });
        }
    }

    /// The dashboard's view of the job right now.
    pub fn snapshot(&self) -> JobSnapshot {
        let now = (self.clock)();
        // Once the run is over (run_end seen, finished or cancelled) a phase
        // that never started is not "en cola" any more: it is dropped.
        let over = !self.running || self.run_ended;
        let phases: Vec<PhaseProgress> = self
            .phases
            .iter()
            .filter(|t| !(over && t.untouched()))
            .map(|t| {
                let mut progress = t.progress.clone();
                progress.tiempo_ms = u64::try_from(t.busy_at(now).as_millis()).unwrap_or(u64::MAX);
                progress
            })
            .collect();
        JobSnapshot {
            id: self.id.clone(),
            tipo: self.kind.clone(),
            corriendo: self.running,
            cancelable: self.running,
            iniciado_en: rfc3339(self.started_at),
            terminado_en: self.ended_at.map_or_else(|| ZERO_TIME.to_owned(), rfc3339),
            error: self.error.clone(),
            salida: self.synthetic_output(&phases),
            estadisticas: self.stats.clone(),
            radicadas: self.radicacion_done,
            radicacion_pasos: self.radicacion_steps.clone(),
            convenio: self.convenio.clone(),
            instancia_servidor: self.instance.clone(),
            fases: phases,
        }
    }

    fn synthetic_output(&self, phases: &[PhaseProgress]) -> String {
        let mut manual = self.prep_manual + phases.iter().map(|p| p.revision_manual).sum::<usize>();
        if let (true, Some(stats)) = (self.run_ended, &self.stats) {
            manual = stats.revision_manual;
        }
        synthetic_log::build(&SyntheticInput {
            kind: &self.kind,
            cargue: &self.cargue,
            plan: &self.plan,
            started_at: self.started_at,
            ended_at: self.ended_at,
            now: Local::now(),
            running: self.running,
            cancelled: self.cancelled,
            error: &self.error,
            convenio: self.convenio.as_ref(),
            stats: self.stats.as_ref(),
            phases,
            run_ended: self.run_ended,
            manual,
            errors: &self.errors,
            errors_more: self.error_total.saturating_sub(self.errors.len()),
        })
    }
}

/// The reason an event carries for a failure.
fn error_detail(e: &Event) -> &str {
    if e.error.is_empty() {
        &e.reason
    } else {
        &e.error
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn ev(kind: &str) -> Event {
        Event {
            kind: kind.to_owned(),
            ..Event::default()
        }
    }

    fn step(phase: &str, invoice: &str, status: &str) -> Event {
        Event {
            phase: phase.to_owned(),
            step: format!("{phase} label"),
            invoice: invoice.to_owned(),
            status: status.to_owned(),
            ..ev("invoice_step")
        }
    }

    fn enter(phase: &str, invoice: &str) -> Event {
        Event {
            phase: phase.to_owned(),
            step: format!("{phase} label"),
            invoice: invoice.to_owned(),
            ..ev("invoice_enter")
        }
    }

    /// A clock the test advances by hand.
    fn fake_clock() -> (Clock, Arc<Mutex<Duration>>) {
        let base = Instant::now();
        let offset = Arc::new(Mutex::new(Duration::ZERO));
        let shared = offset.clone();
        let clock: Clock = Arc::new(move || base + *shared.lock().expect("clock"));
        (clock, offset)
    }

    fn state() -> (JobState, Arc<Mutex<Duration>>) {
        let (clock, offset) = fake_clock();
        (JobState::new("job-1", "proceso run", "abc", clock), offset)
    }

    fn advance(offset: &Arc<Mutex<Duration>>, secs: u64) {
        *offset.lock().expect("clock") += Duration::from_secs(secs);
    }

    #[test]
    fn convenio_events_fill_the_strip_and_the_preparation_counts() {
        let (mut job, _) = state();
        job.observe(&ev("convenio_start"));
        assert_eq!(job.snapshot().convenio.expect("strip").estado, "preparando");
        job.observe(&Event {
            evaluated: 12,
            total: 20,
            ..ev("convenio_progress")
        });
        let strip = job.snapshot().convenio.expect("strip");
        assert_eq!((strip.evaluadas, strip.total), (12, 20));
        job.observe(&Event {
            status: "OK".into(),
            done: 10,
            excluded: 1,
            manual: 1,
            evaluated: 12,
            limit: 10,
            ..ev("convenio")
        });
        let snap = job.snapshot();
        let strip = snap.convenio.expect("strip");
        assert_eq!(strip.estado, "ok");
        assert_eq!(
            (strip.preparadas, strip.excluidas, strip.revision_manual),
            (10, 1, 1)
        );
        assert_eq!(strip.evaluadas, 12, "the walk's own progress is kept");
        let stats = snap.estadisticas.expect("stats");
        assert_eq!((stats.evaluadas, stats.limite), (12, 10));
        assert_eq!((stats.excluidas, stats.revision_manual), (1, 1));
        // A late progress event never reopens the finished step.
        job.observe(&Event {
            evaluated: 1,
            total: 2,
            ..ev("convenio_progress")
        });
        assert_eq!(job.snapshot().convenio.expect("strip").estado, "ok");
    }

    #[test]
    fn a_failed_convenio_event_is_an_error_strip_and_not_a_result() {
        let (mut job, _) = state();
        job.observe(&Event {
            status: "FAILED".into(),
            error: "no hay nota técnica".into(),
            done: 99,
            ..ev("convenio")
        });
        let snap = job.snapshot();
        let strip = snap.convenio.expect("strip");
        assert_eq!(strip.estado, "error");
        assert_eq!(strip.mensaje, "no hay nota técnica");
        assert!(snap.estadisticas.is_none());
    }

    #[test]
    fn run_events_tally_the_stats_and_run_end_adds_the_preparation_counts() {
        let (mut job, _) = state();
        job.observe(&Event {
            status: "OK".into(),
            done: 3,
            excluded: 2,
            manual: 1,
            evaluated: 6,
            ..ev("convenio")
        });
        job.observe(&Event {
            total: 3,
            ..ev("run_start")
        });
        let stats = job.snapshot().estadisticas.expect("stats");
        assert_eq!((stats.esperadas, stats.procesadas), (3, 3));
        assert_eq!((stats.excluidas, stats.revision_manual), (2, 1));
        job.observe(&Event {
            done: 1,
            excluded: 1,
            ..ev("convenio_more")
        });
        job.observe(&ev("invoice_replace"));
        job.observe(&Event {
            done: 2,
            failed: 1,
            manual: 1,
            excluded: 1,
            total: 4,
            replaced: 1,
            ..ev("run_end")
        });
        let stats = job.snapshot().estadisticas.expect("stats");
        assert_eq!((stats.terminadas, stats.con_errores), (2, 1));
        assert_eq!((stats.excluidas, stats.revision_manual), (4, 2));
        assert_eq!(stats.gestionadas, 5, "done + failed + excluded + manual");
        assert_eq!((stats.procesadas, stats.reemplazadas), (4, 1));
        assert_eq!(
            stats.evaluadas, 6,
            "convenio_more adds on top of the first round"
        );
    }

    #[test]
    fn run_end_without_replacement_keeps_the_expected_as_processed() {
        let (mut job, _) = state();
        job.observe(&Event {
            total: 7,
            ..ev("run_start")
        });
        job.observe(&Event {
            done: 7,
            ..ev("run_end")
        });
        let stats = job.snapshot().estadisticas.expect("stats");
        assert_eq!(
            (stats.esperadas, stats.procesadas, stats.reemplazadas),
            (7, 7, 0)
        );
    }

    #[test]
    fn phases_count_busy_wall_time_once_for_overlapping_invoices() {
        let (mut job, offset) = state();
        job.observe(&enter("fev", "A"));
        advance(&offset, 2);
        job.observe(&enter("fev", "B"));
        advance(&offset, 3);
        job.observe(&step("fev", "A", "OK"));
        advance(&offset, 5);
        let snap = job.snapshot();
        assert_eq!(snap.fases[0].en_curso, 1);
        assert_eq!(snap.fases[0].tiempo_ms, 10_000, "the open interval counts");
        job.observe(&step("fev", "B", "MANUAL_REVIEW"));
        advance(&offset, 50);
        let snap = job.snapshot();
        let fev = &snap.fases[0];
        assert_eq!(
            (fev.terminadas, fev.revision_manual, fev.en_curso),
            (1, 1, 0)
        );
        assert_eq!(fev.tiempo_ms, 10_000, "an idle phase stops counting");
    }

    #[test]
    fn step_outcomes_go_to_their_own_counter_and_a_skip_counts_nowhere() {
        let (mut job, _) = state();
        for (invoice, status) in [
            ("A", "OK"),
            ("B", "EXCLUDED"),
            ("C", "FAILED"),
            ("D", "SKIPPED"),
        ] {
            job.observe(&enter("rips", invoice));
            job.observe(&step("rips", invoice, status));
        }
        let snap = job.snapshot();
        let rips = &snap.fases[0];
        assert_eq!(
            (
                rips.terminadas,
                rips.excluidas,
                rips.con_errores,
                rips.revision_manual
            ),
            (1, 1, 1, 0)
        );
        assert_eq!(rips.en_curso, 0);
        assert_eq!(rips.etiqueta, "rips label");
    }

    #[test]
    fn radicacion_batches_feed_its_meter_and_a_failed_one_is_one_error() {
        let (mut job, _) = state();
        job.observe(&Event {
            step: "radicacion run".into(),
            ..ev("radicacion_enter")
        });
        job.observe(&Event {
            step: "radicacion run".into(),
            ..ev("radicacion_enter")
        });
        job.observe(&Event {
            step: "radicacion run".into(),
            status: "OK".into(),
            batch: 2,
            duration_ms: 1500,
            ..ev("radicacion")
        });
        job.observe(&Event {
            step: "radicacion run".into(),
            status: "FAILED".into(),
            batch: 3,
            error: "boom".into(),
            invoices: vec!["A".into(), "B".into(), "C".into()],
            ..ev("radicacion")
        });
        job.observe(&Event {
            step: "radicacion run".into(),
            status: "SKIPPED".into(),
            batch: 1,
            ..ev("radicacion")
        });
        let snap = job.snapshot();
        let rad = snap
            .fases
            .iter()
            .find(|p| p.fase == "radicacion")
            .expect("rad");
        assert_eq!((rad.terminadas, rad.con_errores, rad.en_curso), (2, 3, 0));
        assert_eq!(rad.tiempo_ms, 1500);
        assert_eq!(snap.radicadas, 2);
        assert!(
            snap.salida.is_empty() || !snap.salida.contains("Error ·"),
            "{}",
            snap.salida
        );
        job.observe(&ev("run_end"));
        job.running = false;
        let out = job.snapshot().salida;
        assert!(
            out.contains("Error · lote de 3 facturas · radicacion run · boom"),
            "{out}"
        );
    }

    #[test]
    fn radicacion_steps_count_each_invoice_where_it_is() {
        let (mut job, _) = state();
        let at = |step: i16, status: &str| Event {
            invoice: "F1".into(),
            phase: format!("radicacion_{step}"),
            status: status.into(),
            ..ev("radicacion_step")
        };
        assert!(job.snapshot().radicacion_pasos.is_empty());
        job.observe(&at(1, "ENTER"));
        job.observe(&at(1, "DONE"));
        job.observe(&at(2, "ENTER"));
        job.observe(&at(2, "BACK"));
        job.observe(&at(1, "ENTER"));
        job.observe(&at(1, "DONE"));
        job.observe(&at(2, "ENTER"));
        job.observe(&at(2, "DONE"));
        job.observe(&at(3, "ENTER"));
        job.observe(&at(3, "FAILED"));
        let pasos = job.snapshot().radicacion_pasos;
        let counts: Vec<_> = pasos
            .iter()
            .map(|p| (p.fase.as_str(), p.en_curso, p.terminadas, p.con_errores))
            .collect();
        assert_eq!(
            counts,
            [
                ("radicacion_1", 0, 2, 0),
                ("radicacion_2", 0, 1, 0),
                ("radicacion_3", 0, 0, 1),
                ("radicacion_4", 0, 0, 0)
            ]
        );
        assert_eq!(pasos[0].etiqueta, "Radicación · paso 1: ZIP XML/PDF");
    }

    #[test]
    fn errors_are_distinct_capped_at_five_and_counted() {
        let (mut job, _) = state();
        for n in 0..8 {
            let invoice = format!("F{n}");
            job.observe(&enter("cuv", &invoice));
            job.observe(&Event {
                reason: format!("reason {n}"),
                ..step("cuv", &invoice, "FAILED")
            });
        }
        // The same failure again is not a new error.
        job.observe(&enter("cuv", "F0"));
        job.observe(&Event {
            reason: "reason 0".into(),
            ..step("cuv", "F0", "FAILED")
        });
        job.observe(&enter("cuv", "G"));
        job.observe(&step("cuv", "G", "FAILED"));
        job.observe(&ev("run_end"));
        let out = job.snapshot().salida;
        assert_eq!(out.matches("\nError · ").count(), 5, "{out}");
        assert!(out.contains("+4 errores más"), "{out}");
        assert!(
            !out.contains("Error · G"),
            "past the cap it is only counted"
        );
    }

    #[test]
    fn an_empty_reason_reads_sin_detalle() {
        let (mut job, _) = state();
        job.observe(&enter("cuv", "G"));
        job.observe(&step("cuv", "G", "FAILED"));
        job.observe(&ev("run_end"));
        assert!(
            job.snapshot()
                .salida
                .contains("Error · G · cuv label · sin detalle")
        );
    }

    #[test]
    fn seeded_phases_are_listed_in_plan_order_and_dropped_when_never_run() {
        let (mut job, _) = state();
        job.observe(&enter("cuv", "A")); // an early event creates its entry first
        job.seed_phases(&[
            ("thefactory".into(), "thefactory descargar".into()),
            ("cuv".into(), "cuv generar".into()),
            ("soporte".into(), "soporte generar".into()),
        ]);
        let snap = job.snapshot();
        let keys: Vec<&str> = snap.fases.iter().map(|p| p.fase.as_str()).collect();
        assert_eq!(keys, ["thefactory", "cuv", "soporte"], "queued phases show");
        assert_eq!(snap.fases[1].en_curso, 1, "the early tally is kept");
        job.observe(&step("cuv", "A", "OK"));
        job.running = false;
        job.close_busy();
        let snap = job.snapshot();
        let keys: Vec<&str> = snap.fases.iter().map(|p| p.fase.as_str()).collect();
        assert_eq!(
            keys,
            ["cuv"],
            "phases that never ran are dropped at the end"
        );
    }

    #[test]
    fn the_snapshot_json_follows_go_omission_rules() {
        let (job, _) = state();
        let json = serde_json::to_value(job.snapshot()).expect("json");
        let object = json.as_object().expect("object");
        for key in [
            "id",
            "tipo",
            "corriendo",
            "cancelable",
            "iniciado_en",
            "terminado_en",
            "salida",
            "instancia_servidor",
        ] {
            assert!(object.contains_key(key), "{key}");
        }
        for key in ["error", "estadisticas", "fases", "radicadas", "convenio"] {
            assert!(!object.contains_key(key), "{key} is omitted when empty");
        }
        assert_eq!(object["terminado_en"], ZERO_TIME);
        assert_eq!(object["cancelable"], true);
        let idle = serde_json::to_value(JobSnapshot::idle("9f2c")).expect("json");
        assert_eq!(
            idle,
            serde_json::json!({
                "id": "", "tipo": "", "corriendo": false, "cancelable": false,
                "iniciado_en": ZERO_TIME, "terminado_en": ZERO_TIME, "salida": "",
                "instancia_servidor": "9f2c"
            })
        );
    }
}
