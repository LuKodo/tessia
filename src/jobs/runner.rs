//! The job runner (Go `jobRunner`): at most one job at a time per process,
//! each on its own thread, with a per-job cancel and a clean shutdown.
//!
//! * `job-N` ids continue after the highest persisted one.
//! * A panic in the work becomes the job error `panic: <value>`.
//! * Cancel raises [`crate::shutdown::cancel_job`], the flag every phase
//!   already polls through [`crate::shutdown::requested`]; the job ends with the
//!   error `cancelado por el operador`. The flag is lowered when the job ends
//!   (and before the next one starts), so it is only meaningful because exactly
//!   one job runs at a time.
//! * [`JobRunner::close`] rejects new starts, cancels the running job and waits
//!   for it.
//!
//! The work of a job is a closure over the [`AppContext`] and the proceso
//! [`Events`] sink; the runner feeds the sink's observer into the job's
//! [`JobState`]. What each kind of job runs is a [`JobBackend`]: the real one
//! calls the phases, tests plug in fakes.

use std::any::Any;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use anyhow::Result;
use chrono::{Local, NaiveDate};

use super::history::{self, HistoryEntry, HistoryFile, MAX_ENTRIES};
use super::snapshot::{Clock, JobSnapshot, JobState, system_clock};
use crate::context::AppContext;
use crate::phases::common::PhaseRequest;
use crate::phases::conciliar::{self, ConciliarOptions};
use crate::phases::convenio_descargar;
use crate::phases::proceso::{self, Events, Mode, PhaseRunner, RunConfig};

/// The job error a cancel leaves (the UI also matches `cancelad[oa]`).
pub const CANCELLED_ERROR: &str = "cancelado por el operador";

/// The work of one job.
pub type Work = Box<dyn FnOnce(&AppContext, &Events) -> Result<()> + Send + 'static>;

type ProcesoFn = Arc<dyn Fn(&AppContext, &RunConfig, &Events) -> Result<()> + Send + Sync>;
type DownloadFn = Arc<dyn Fn(&AppContext, NaiveDate, NaiveDate) -> Result<()> + Send + Sync>;
type ConciliarFn = Arc<dyn Fn(&AppContext, &PhaseRequest) -> Result<()> + Send + Sync>;
type ConsoleFn = Arc<dyn Fn() -> Box<dyn Write + Send> + Send + Sync>;

/// What each kind of job executes, and where the verbose stream of a
/// `proceso run` goes (the process console, as in Go; the dashboard only gets
/// the synthetic log).
#[derive(Clone)]
pub struct JobBackend {
    /// `proceso run` with a resolved configuration.
    pub proceso: ProcesoFn,
    /// `convenio descargar` for a date range.
    pub download: DownloadFn,
    /// `conciliar --invoices <one> <cargue>`.
    pub conciliar: ConciliarFn,
    /// The writer of the verbose event stream.
    pub console: ConsoleFn,
}

impl JobBackend {
    /// The real phases; the verbose stream goes to stdout.
    pub fn real() -> Self {
        Self {
            proceso: Arc::new(|ctx, cfg, events| {
                let runner = PhaseRunner::real(cfg);
                proceso::execute(ctx, cfg, &runner, events).map(|_| ())
            }),
            download: Arc::new(|ctx, from, to| {
                let outcome = convenio_descargar::download_range(ctx, from, to)?;
                println!(
                    "convenio saved: {} ({})",
                    outcome.cargue_id, outcome.object_key
                );
                println!("{}", outcome.status.describe());
                Ok(())
            }),
            conciliar: Arc::new(|ctx, request| {
                conciliar::run_request(ctx, request, &ConciliarOptions::standalone())
            }),
            console: Arc::new(|| Box::new(std::io::stdout())),
        }
    }

    /// The same backend with the verbose stream discarded.
    #[must_use]
    pub fn quiet(mut self) -> Self {
        self.console = Arc::new(|| Box::new(std::io::sink()));
        self
    }
}

/// What the synthetic log and the phase cards need to know about a run before
/// its first event.
#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    pub cargue: Option<String>,
    /// Labels of the phases the run executes (the log's start line).
    pub plan: Vec<String>,
    /// `(fase, etiqueta)` of the phase cards to list from the start.
    pub phases: Vec<(String, String)>,
}

/// Why a job did not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    /// Another job is running (its kind and start time `HH:MM:SS`).
    Running { kind: String, started: String },
    /// The runner is shutting down.
    Stopping,
    /// The job thread could not be created.
    Spawn(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::Running { kind, started } => write!(
                f,
                "ya hay un proceso en curso ({kind}, iniciado {started}): espera a que termine"
            ),
            StartError::Stopping => f.write_str("job runner is shutting down"),
            StartError::Spawn(err) => write!(f, "no se pudo iniciar el proceso: {err}"),
        }
    }
}

impl std::error::Error for StartError {}

/// A cancel with nothing to cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoJobRunning;

impl std::fmt::Display for NoJobRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no hay ningun proceso en curso")
    }
}

impl std::error::Error for NoJobRunning {}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// One job: its live state and a latch that opens when its thread is done.
struct Job {
    state: Mutex<JobState>,
    finished: Mutex<bool>,
    finished_cv: Condvar,
}

impl Job {
    fn wait(&self) {
        let mut done = lock(&self.finished);
        while !*done {
            done = self
                .finished_cv
                .wait(done)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    fn open_latch(&self) {
        *lock(&self.finished) = true;
        self.finished_cv.notify_all();
    }

    /// Marks the job cancelled when it is running; false when it already ended.
    fn cancel(&self) -> bool {
        let mut state = lock(&self.state);
        if !state.running {
            return false;
        }
        state.cancelled = true;
        crate::shutdown::cancel_job();
        true
    }
}

struct Inner {
    current: Option<Arc<Job>>,
    seq: u64,
    /// Newest first.
    entries: Vec<HistoryEntry>,
    stopping: bool,
}

struct Shared {
    inner: Mutex<Inner>,
    history: HistoryFile,
}

impl Shared {
    /// Prepends the entry to the in-memory history and appends it to the file.
    /// A write failure only logs: the audit trail must never break the dashboard.
    fn record(&self, entry: HistoryEntry) {
        {
            let mut inner = lock(&self.inner);
            inner.entries.insert(0, entry.clone());
            inner.entries.truncate(MAX_ENTRIES);
        }
        if let Err(err) = self.history.append(&entry) {
            tracing::warn!(error = %format!("{err:#}"), "could not append to the job history file");
        }
    }
}

/// Runs at most one job at a time: two concurrent `proceso run` or
/// `convenio descargar` would race on the same cargues and phase claims.
pub struct JobRunner {
    pub ctx: AppContext,
    backend: JobBackend,
    instance: String,
    clock: Clock,
    shared: Arc<Shared>,
}

impl JobRunner {
    /// A runner that keeps its history under the OS user data directory.
    pub fn new(ctx: AppContext) -> Self {
        Self::with_history_dir(ctx, history::default_dir())
    }

    /// A runner that keeps its history in `<dir>/.tessia-serve-jobs.jsonl`
    /// (memory only when `dir` is `None`). The persisted history is loaded now
    /// and the ids continue after the highest one in it.
    pub fn with_history_dir(ctx: AppContext, dir: Option<PathBuf>) -> Self {
        let history = HistoryFile::new(dir);
        let loaded = history.load();
        Self {
            ctx,
            backend: JobBackend::real(),
            instance: format!("{:016x}", rand::random::<u64>()),
            clock: system_clock(),
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner {
                    current: None,
                    seq: loaded.max_seq,
                    entries: loaded.entries,
                    stopping: false,
                }),
                history,
            }),
        }
    }

    /// Replaces what the jobs execute (tests).
    #[must_use]
    pub fn with_backend(mut self, backend: JobBackend) -> Self {
        self.backend = backend;
        self
    }

    /// Replaces the clock busy times are measured with (tests).
    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn backend(&self) -> &JobBackend {
        &self.backend
    }

    /// Random hex that identifies this process: a client that sees it change
    /// knows the server restarted.
    pub fn instance_id(&self) -> &str {
        &self.instance
    }

    /// Past jobs, most recently finished first.
    pub fn history(&self) -> Vec<HistoryEntry> {
        lock(&self.shared.inner).entries.clone()
    }

    /// The most recently started job (running or finished), or the idle
    /// snapshot (with the instance id) when none has run in this process.
    pub fn current(&self) -> JobSnapshot {
        let job = lock(&self.shared.inner).current.clone();
        match job {
            Some(job) => lock(&job.state).snapshot(),
            None => JobSnapshot::idle(&self.instance),
        }
    }

    /// Starts `work` as a new job of `kind` unless one is running.
    pub fn start(
        &self,
        kind: &str,
        options: StartOptions,
        work: Work,
    ) -> Result<JobSnapshot, StartError> {
        let mut inner = lock(&self.shared.inner);
        if inner.stopping {
            return Err(StartError::Stopping);
        }
        if let Some(job) = &inner.current {
            let state = lock(&job.state);
            if state.running {
                return Err(StartError::Running {
                    kind: state.kind.clone(),
                    started: state.started_at.format("%H:%M:%S").to_string(),
                });
            }
        }
        let id = format!("job-{}", inner.seq + 1);
        let mut state = JobState::new(&id, kind, &self.instance, self.clock.clone());
        if let Some(cargue) = &options.cargue {
            state.describe(cargue, options.plan);
        }
        state.seed_phases(&options.phases);
        let job = Arc::new(Job {
            state: Mutex::new(state),
            finished: Mutex::new(false),
            finished_cv: Condvar::new(),
        });
        // A previous cancel must not leak into this job.
        crate::shutdown::reset_job();
        self.spawn(&id, &job, work)?;
        inner.seq += 1;
        inner.current = Some(job.clone());
        let snapshot = lock(&job.state).snapshot();
        Ok(snapshot)
    }

    fn spawn(&self, id: &str, job: &Arc<Job>, work: Work) -> Result<(), StartError> {
        let (ctx, shared) = (self.ctx.clone(), self.shared.clone());
        let (job, console) = (job.clone(), self.backend.console.clone());
        std::thread::Builder::new()
            .name(id.to_owned())
            .spawn(move || {
                let observed = job.clone();
                let events = Events::new(Mode::Text, console(), false)
                    .with_tap(move |event| lock(&observed.state).observe(event));
                let outcome = catch_unwind(AssertUnwindSafe(|| work(&ctx, &events)));
                let error = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(err)) => Some(format!("{err:#}")),
                    Err(payload) => Some(format!("panic: {}", panic_text(payload.as_ref()))),
                };
                finish(&shared, &job, error);
            })
            .map(|_| ())
            .map_err(|err| StartError::Spawn(err.to_string()))
    }

    /// Asks the running job to stop; it ends with [`CANCELLED_ERROR`].
    pub fn cancel(&self) -> Result<(), NoJobRunning> {
        let job = lock(&self.shared.inner).current.clone();
        match job {
            Some(job) if job.cancel() => Ok(()),
            _ => Err(NoJobRunning),
        }
    }

    /// Rejects new jobs, cancels the running one and waits until it has stopped
    /// using the shared resources.
    pub fn close(&self) {
        let job = {
            let mut inner = lock(&self.shared.inner);
            inner.stopping = true;
            inner.current.clone()
        };
        if let Some(job) = job {
            job.cancel();
            job.wait();
        }
    }
}

/// The job's only cleanup path: it runs whether the work returned, failed or
/// panicked, so an error and a history entry are always recorded.
fn finish(shared: &Shared, job: &Job, error: Option<String>) {
    let snapshot = {
        let mut state = lock(&job.state);
        state.running = false;
        state.ended_at = Some(Local::now());
        state.close_busy();
        if let Some(err) = error {
            state.error = if state.cancelled {
                CANCELLED_ERROR.to_owned()
            } else {
                err
            };
        }
        // Under the state lock: a cancel only acts on a running job, so none can
        // raise the flag after this reset.
        crate::shutdown::reset_job();
        state.snapshot()
    };
    shared.record(HistoryEntry::from_snapshot(&snapshot));
    job.open_latch();
}

fn panic_text(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "valor desconocido".to_owned()
    }
}
