//! Helpers of `api_jobs.rs`: an `ApiState` over a throwaway database with a
//! fake job backend, and the gate the fake jobs block on.
#![allow(dead_code)]

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::NaiveDate;
use serde_json::Value;
use tessia::api::{ApiState, Request, Response};
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::jobs::{JobBackend, JobRunner};
use tessia::nt::MemoryNt;
use tessia::phases::common::PhaseRequest;
use tessia::phases::proceso::{Events, RunConfig};

/// The job cancel flag is process-wide (one job at a time per process), so the
/// tests that run jobs take turns.
static SERIAL: Mutex<()> = Mutex::new(());

type ProcesoFake = Arc<dyn Fn(&AppContext, &RunConfig, &Events) -> Result<()> + Send + Sync>;
type DownloadFake = Arc<dyn Fn(NaiveDate, NaiveDate) -> Result<()> + Send + Sync>;
type ConciliarFake = Arc<dyn Fn(&PhaseRequest) -> Result<()> + Send + Sync>;

/// What the fake jobs do and what they were asked.
#[derive(Default)]
pub struct Fakes {
    pub proceso: Option<ProcesoFake>,
    pub download: Option<DownloadFake>,
    pub conciliar: Option<ConciliarFake>,
}

impl Fakes {
    pub fn proceso(
        mut self,
        run: impl Fn(&AppContext, &RunConfig, &Events) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.proceso = Some(Arc::new(run));
        self
    }

    pub fn download(
        mut self,
        run: impl Fn(NaiveDate, NaiveDate) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.download = Some(Arc::new(run));
        self
    }

    pub fn conciliar(
        mut self,
        run: impl Fn(&PhaseRequest) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.conciliar = Some(Arc::new(run));
        self
    }
}

/// What the fake backend was called with.
#[derive(Default)]
pub struct Calls {
    pub proceso: Mutex<Vec<RunConfig>>,
    pub download: Mutex<Vec<(NaiveDate, NaiveDate)>>,
    pub conciliar: Mutex<Vec<PhaseRequest>>,
}

pub struct Env {
    pub db: TestDb,
    pub ctx: AppContext,
    pub state: ApiState,
    pub jobs: Arc<JobRunner>,
    pub calls: Arc<Calls>,
    pub history_dir: tempfile::TempDir,
    _serial: MutexGuard<'static, ()>,
}

pub fn env(fakes: Fakes) -> Env {
    env_with_nt(fakes, MemoryNt::new())
}

pub fn env_with_nt(fakes: Fakes, nt: MemoryNt) -> Env {
    let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db).with_nt(Arc::new(nt));
    let history_dir = tempfile::tempdir().expect("history dir");
    let calls = Arc::new(Calls::default());
    let jobs = Arc::new(runner(&ctx, &history_dir, fakes, &calls));
    let state = ApiState::new(ctx.clone(), db.pool(), jobs.clone());
    Env {
        db,
        ctx,
        state,
        jobs,
        calls,
        history_dir,
        _serial: serial,
    }
}

/// A runner over `history_dir` whose jobs call the fakes.
pub fn runner(
    ctx: &AppContext,
    history_dir: &tempfile::TempDir,
    fakes: Fakes,
    calls: &Arc<Calls>,
) -> JobRunner {
    let mut backend = JobBackend::real().quiet();
    let record = calls.clone();
    let proceso = fakes.proceso.clone();
    backend.proceso = Arc::new(move |ctx, cfg, events| {
        record.proceso.lock().expect("calls").push(cfg.clone());
        proceso.as_ref().map_or(Ok(()), |run| run(ctx, cfg, events))
    });
    let record = calls.clone();
    let download = fakes.download.clone();
    backend.download = Arc::new(move |_, from, to| {
        record.download.lock().expect("calls").push((from, to));
        download.as_ref().map_or(Ok(()), |run| run(from, to))
    });
    let record = calls.clone();
    let conciliar = fakes.conciliar.clone();
    backend.conciliar = Arc::new(move |_, request| {
        record
            .conciliar
            .lock()
            .expect("calls")
            .push(request.clone());
        conciliar.as_ref().map_or(Ok(()), |run| run(request))
    });
    JobRunner::with_history_dir(ctx.clone(), Some(history_dir.path().to_path_buf()))
        .with_backend(backend)
}

impl Env {
    pub fn get(&self, path: &str) -> Response {
        self.state.handle(Request::new("GET", path))
    }

    pub fn post(&self, path: &str, body: &str) -> Response {
        self.state
            .handle(Request::new("POST", path).with_body(body.as_bytes().to_vec()))
    }

    /// `GET /api/jobs/current` once the job has finished.
    pub fn wait_done(&self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let snapshot = body(&self.get("/api/jobs/current"));
            if snapshot["corriendo"] == false {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "the job did not finish: {snapshot}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The `error` of a failed request.
    pub fn error_of(&self, response: &Response) -> String {
        body(response)["error"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
}

pub fn body(response: &Response) -> Value {
    serde_json::from_slice(&response.body).expect("json body")
}

pub fn error(response: &Response) -> (u16, String) {
    (
        response.status,
        body(response)["error"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    )
}

/// A latch the fake jobs block on until the test opens it or the job is
/// cancelled (the way the real phases poll `shutdown::requested`).
#[derive(Clone, Default)]
pub struct Gate(Arc<GateState>);

#[derive(Default)]
struct GateState {
    flags: Mutex<(bool, bool)>, // (reached, open)
    changed: Condvar,
}

impl Gate {
    /// Called by the job: announces it is waiting, then waits.
    pub fn wait(&self) {
        let mut flags = self.0.flags.lock().expect("gate");
        flags.0 = true;
        self.0.changed.notify_all();
        while !flags.1 && !tessia::shutdown::requested() {
            let (next, _) = self
                .0
                .changed
                .wait_timeout(flags, Duration::from_millis(5))
                .expect("gate");
            flags = next;
        }
    }

    /// Called by the test: blocks until the job is waiting.
    pub fn reached(&self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut flags = self.0.flags.lock().expect("gate");
        while !flags.0 {
            assert!(Instant::now() < deadline, "the job never started");
            let (next, _) = self
                .0
                .changed
                .wait_timeout(flags, Duration::from_millis(5))
                .expect("gate");
            flags = next;
        }
    }

    pub fn open(&self) {
        self.0.flags.lock().expect("gate").1 = true;
        self.0.changed.notify_all();
    }
}
