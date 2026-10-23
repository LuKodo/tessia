//! The per-invoice filing pipeline (Go `internal/radicacion/pipeline`).
//!
//! Each step is a stage with its own queues and workers:
//!
//! ```text
//! send queue -> collector -> micro-batch -> senders -> upload -> confirm queue
//!                (BatchMax or BatchWait,     (2)                      |
//!                 one shared id_cargue)                        confirmers (16)
//!   confirmed -> mark filed, enqueue into the NEXT stage at once  <---+
//!   pending   -> poll again later (backoff)
//!   failed    -> re-send (quick for transient messages, else RetryInterval);
//!                step 2 "invoice missing" goes back to stage 1 once
//!   note / rejected / blocked -> leaves the pipeline
//! ```
//!
//! There are no wave barriers: an invoice never waits for others. The state is
//! persisted after every transition, so an interrupted run resumes where it
//! stopped (`SENT` invoices are polled again with the same `upload_id`, never
//! re-sent).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use crossbeam_channel::{Receiver, Sender, bounded, select, unbounded};

use super::api::{ApiError, RadApi};
use super::confirm::{Evidence, Outcome, Verdict, evaluate};
use super::files::{Files, Found};
use super::model::{Load, UploadFile};
use super::progress::{ProgressSink, StepProgress};
use super::rules::{
    check_value, requires_step1_reload, retryable_upload_message, still_processing, upload_lost,
};
use super::timings::{Timings, queue, sleep};
use super::values::Values;
use crate::domain::radicacion::{RadState, RadStatus, Step};
use crate::stores::radicacion::RadicacionStore;
use crate::util::par;

/// A wait setting where a negative value has its own meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Wait {
    /// Not set: use the default.
    #[default]
    Default,
    /// Negative on the command line: switched off.
    Disabled,
    Fixed(Duration),
}

/// Tunes the pipeline. Zero values use the defaults of [`Config::resolved`].
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Issuer NIT (CRC file names).
    pub nit: String,
    /// Files per micro-batch (default 200, hard cap 400).
    pub batch_max: usize,
    pub batch_wait: Duration,
    /// Per stage.
    pub send_workers: usize,
    pub confirm_workers: usize,
    /// Invoices of a batch pre-checked at once (1 = one by one).
    pub prepare_concurrency: usize,
    /// Caps each portal pre-check of steps 3 and 4 (never confirmations).
    pub precheck_timeout: Wait,
    pub first_poll_delay: Duration,
    /// Doubles per poll up to `poll_max_interval`.
    pub poll_interval: Duration,
    pub poll_max_interval: Duration,
    /// Transient failures are re-sent this many times after `quick_retry_delay`.
    pub quick_retries: i32,
    pub quick_retry_delay: Duration,
    /// Delay of the other re-sends (the legacy 10-minute round).
    pub retry_interval: Duration,
    /// Polls with the TXT loaded but filter-xml not ACTIVE before re-sending.
    pub step2_xml_polls: i32,
    /// A step 4 upload whose "fecha" is older and not loaded is re-sent.
    pub crc_lost_after: Duration,
    /// Wait after re-sending step 1 before retrying step 2.
    pub reload_wait: Wait,
    /// Bounds the whole run.
    pub deadline: Duration,
    /// Rejects a step after this many uploads (0 = unlimited).
    pub max_sends: i32,
    /// Runs a single stage.
    pub only_step: Option<Step>,
    /// How long a cancelled run waits for in-flight calls before returning.
    pub shutdown_grace: Duration,
}

impl Config {
    /// The configuration with every default applied.
    pub fn resolved(mut self) -> Config {
        fn or(value: &mut Duration, default: Duration) {
            if value.is_zero() {
                *value = default;
            }
        }
        if self.batch_max == 0 {
            self.batch_max = 200;
        }
        self.batch_max = self.batch_max.min(400); // Mutual Ser accepts at most 400 files per upload
        or(&mut self.batch_wait, Duration::from_secs(2));
        if self.send_workers == 0 {
            self.send_workers = 2;
        }
        if self.confirm_workers == 0 {
            self.confirm_workers = 16;
        }
        if self.prepare_concurrency == 0 {
            self.prepare_concurrency = 20;
        }
        or(&mut self.first_poll_delay, Duration::from_secs(3));
        or(&mut self.poll_interval, Duration::from_secs(5));
        or(&mut self.poll_max_interval, Duration::from_secs(120));
        if self.quick_retries <= 0 {
            self.quick_retries = 3;
        }
        or(&mut self.quick_retry_delay, Duration::from_secs(30));
        or(&mut self.retry_interval, Duration::from_secs(600));
        if self.step2_xml_polls <= 0 {
            self.step2_xml_polls = 5;
        }
        or(&mut self.crc_lost_after, Duration::from_secs(90));
        or(&mut self.deadline, Duration::from_secs(3 * 3600));
        or(&mut self.shutdown_grace, Duration::from_secs(30));
        self
    }

    /// The pre-check cap; `None` = no cap.
    fn precheck_cap(&self) -> Option<Duration> {
        match self.precheck_timeout {
            Wait::Default => Some(Duration::from_secs(10)),
            Wait::Disabled => None,
            Wait::Fixed(d) => Some(d),
        }
    }

    fn reload_delay(&self) -> Duration {
        match self.reload_wait {
            Wait::Default => Duration::from_secs(45),
            Wait::Disabled => Duration::ZERO,
            Wait::Fixed(d) => d,
        }
    }
}

/// Told once per successful upload of a step of an invoice.
pub type UploadedHook = Arc<dyn Fn(&str, Step) + Send + Sync>;

/// Items to start with, items to skip, items already done.
type Loaded = (Vec<Item>, Vec<RadState>, Vec<RadState>);

/// Everything the engine talks to.
pub struct Deps {
    pub api: Arc<dyn RadApi>,
    pub states: RadicacionStore,
    pub files: Arc<dyn Files>,
    /// `None` skips the step 1 value check.
    pub values: Option<Arc<Values>>,
    pub timings: Arc<Timings>,
    pub now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    /// Told once per successful upload of a step of an invoice, after the
    /// progress was recorded. Must be cheap; a panic in it is contained.
    pub on_uploaded: Option<UploadedHook>,
    /// Told about every step transition of every invoice (dashboard progress).
    pub on_progress: Option<ProgressSink>,
}

impl Deps {
    pub fn new(
        api: Arc<dyn RadApi>,
        states: RadicacionStore,
        files: Arc<dyn Files>,
        values: Option<Arc<Values>>,
        timings: Arc<Timings>,
    ) -> Deps {
        Deps {
            api,
            states,
            files,
            values,
            timings,
            now: Arc::new(Utc::now),
            on_uploaded: None,
            on_progress: None,
        }
    }
}

/// What a run did. The lists hold normalized invoice numbers.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Fully filed (including invoices filed before this run).
    pub done: Vec<String>,
    /// Single-stage runs: the stage was confirmed.
    pub advanced: Vec<String>,
    /// Credit note, adjustment note or retry budget exhausted.
    pub rejected: Vec<String>,
    /// Local validation failed (value mismatch, missing file).
    pub blocked: Vec<String>,
    /// An upload is still being processed at the deadline.
    pub in_progress: Vec<String>,
    /// Still pending (failing) at the deadline.
    pub unclassified: Vec<String>,
    /// Not processed in this run (already final or another stage).
    pub skipped: Vec<String>,
    pub deadline_reached: bool,
    pub states: HashMap<String, RadState>,
}

impl Report {
    /// One-line report.
    pub fn summary(&self) -> String {
        let mut parts = vec![
            format!("done={}", self.done.len()),
            format!("advanced={}", self.advanced.len()),
            format!("rejected={}", self.rejected.len()),
            format!("blocked={}", self.blocked.len()),
            format!("in_progress={}", self.in_progress.len()),
            format!("unclassified={}", self.unclassified.len()),
            format!("skipped={}", self.skipped.len()),
        ];
        if self.deadline_reached {
            parts.push("deadline_reached=true".to_owned());
        }
        parts.join(" ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Done,
    Advanced,
    Rejected,
    Blocked,
}

/// One invoice travelling through the pipeline; exactly one thread owns it.
struct Item {
    st: RadState,
    /// The located file of the current step.
    file: Option<UploadFile>,
    /// Consecutive upload errors (network/HTTP).
    send_errors: u32,
    /// When the current upload finished (timings clock; `None` when it was
    /// not made in this run).
    sent_tick: Option<Instant>,
    /// When the item entered a worker queue (timings clock).
    queued_at: Option<Instant>,
    /// This run moved the item into its current step (it confirmed the
    /// previous one): nobody else can have filed the new step in between, so
    /// its pre-upload portal check is redundant until a re-send.
    advanced_here: bool,
}

impl Item {
    fn new(st: RadState) -> Item {
        Item {
            st,
            file: None,
            send_errors: 0,
            sent_tick: None,
            queued_at: None,
            advanced_here: false,
        }
    }
}

fn idx(step: Step) -> usize {
    (step.as_i16() - 1) as usize
}

#[derive(Clone, Copy)]
enum Queue {
    Send(Step),
    Confirm(Step),
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct TimerEntry {
    due: Instant,
    seq: u64,
    queue: Queue,
    item: Item,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        (self.due, self.seq) == (other.due, other.seq)
    }
}
impl Eq for TimerEntry {}
impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.due, self.seq).cmp(&(other.due, other.seq))
    }
}

#[derive(Default)]
struct TimerState {
    heap: BinaryHeap<Reverse<TimerEntry>>,
    seq: u64,
    closed: bool,
}

/// Delivers delayed items to their queues (Go `time.AfterFunc`).
#[derive(Default)]
struct Timer {
    state: Mutex<TimerState>,
    wake: Condvar,
}

impl Timer {
    fn schedule(&self, delay: Duration, queue: Queue, item: Item) {
        let mut state = lock(&self.state);
        state.seq += 1;
        let entry = TimerEntry {
            due: Instant::now() + delay,
            seq: state.seq,
            queue,
            item,
        };
        state.heap.push(Reverse(entry));
        self.wake.notify_all();
    }

    fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        state.heap.clear();
        self.wake.notify_all();
    }

    fn run(&self, shared: &Shared) {
        let mut state = lock(&self.state);
        loop {
            if state.closed {
                return;
            }
            let now = Instant::now();
            match state.heap.peek().map(|Reverse(e)| e.due) {
                Some(due) if due <= now => {
                    let entry = state.heap.pop();
                    drop(state);
                    if let Some(Reverse(entry)) = entry {
                        shared.deliver(entry.queue, entry.item);
                    }
                    state = lock(&self.state);
                }
                Some(due) => {
                    state = self
                        .wake
                        .wait_timeout(state, due - now)
                        .unwrap_or_else(|p| p.into_inner())
                        .0;
                }
                None => {
                    state = self.wake.wait(state).unwrap_or_else(|p| p.into_inner());
                }
            }
        }
    }
}

/// Runs the pipeline.
pub struct Engine {
    cfg: Config,
    deps: Deps,
}

impl Engine {
    pub fn new(cfg: Config, deps: Deps) -> Engine {
        Engine {
            cfg: cfg.resolved(),
            deps,
        }
    }

    /// Files `invoices` of the cargue until all leave the pipeline, the
    /// deadline passes or the process is asked to stop.
    pub fn run(&self, cargue_id: &str, invoices: &[String]) -> Result<Report> {
        let invoices = normalize_all(invoices);
        let mut report = Report::default();
        let (items, skipped, done) = self.load(cargue_id, &invoices)?;
        let (shared, workers) = Shared::start(&self.cfg, &self.deps, &items);
        {
            let mut latest = lock(&shared.latest);
            for it in &items {
                latest.insert(it.st.invoice.clone(), it.st.clone());
            }
        }
        let mut resumed = [0usize; 4];
        for it in items {
            if it.st.status == RadStatus::Sent {
                resumed[idx(it.st.step)] += 1;
            }
            shared.dispatch(it);
        }
        for step in Step::ALL {
            if resumed[idx(step)] > 0 {
                shared.announce_waiting(step, resumed[idx(step)]);
            }
        }
        report.deadline_reached = shared.wait_until_done(self.cfg.deadline);
        shared.cancel();
        join_with_grace(workers, self.cfg.shutdown_grace);
        shared.fill_report(&mut report, &done, &skipped);
        Ok(report)
    }

    /// The starting state of every invoice: the stored state, else a fresh
    /// step 1 seed.
    fn load(&self, cargue_id: &str, invoices: &[String]) -> Result<Loaded> {
        let tm = &self.deps.timings;
        let t0 = tm.now();
        let stored = self.deps.states.load(invoices);
        tm.observe_db(tm.since(t0));
        let stored = stored?;
        let (mut items, mut skipped, mut done) = (Vec::new(), Vec::new(), Vec::new());
        for invoice in invoices {
            let mut st = match stored.get(invoice) {
                Some(st) => st.clone(),
                None => RadState::new(invoice, Step::InvoiceZip),
            };
            if matches!(st.status, RadStatus::Blocked | RadStatus::Rejected) {
                // Operators fix blocked invoices (files, values); try again.
                st.status = RadStatus::Pending;
                st.sends = 0;
                st.polls = 0;
                st.next_attempt_at = None;
            }
            st.folder = cargue_id.to_owned();
            if st.is_done() {
                done.push(st);
            } else if st.is_terminal() || self.cfg.only_step.is_some_and(|only| st.step != only) {
                skipped.push(st);
            } else {
                items.push(Item::new(st));
            }
        }
        Ok((items, skipped, done))
    }
}

/// Normalized, deduplicated, non-empty invoices in order.
fn normalize_all(invoices: &[String]) -> Vec<String> {
    crate::domain::invoice::normalize_all(
        invoices.iter().map(|i| crate::domain::normalize_invoice(i)),
    )
}

/// Joins the workers, giving up after `grace` (in-flight HTTP calls cannot be
/// cancelled; they finish and persist their result on their own).
fn join_with_grace(workers: Vec<JoinHandle<()>>, grace: Duration) {
    let (tx, rx) = bounded::<()>(1);
    std::thread::spawn(move || {
        for worker in workers {
            let _ = worker.join();
        }
        let _ = tx.send(());
    });
    if rx.recv_timeout(grace).is_err() {
        tracing::warn!(
            grace_s = grace.as_secs(),
            "workers still busy after the shutdown grace"
        );
    }
}

/// State shared by every worker of a run.
struct Shared {
    cfg: Config,
    api: Arc<dyn RadApi>,
    states: RadicacionStore,
    files: Arc<dyn Files>,
    values: Option<Arc<Values>>,
    tm: Arc<Timings>,
    now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    hook: Option<UploadedHook>,
    progress: Option<ProgressSink>,
    send_tx: Vec<Sender<Item>>,
    confirm_tx: Vec<Sender<Item>>,
    timer: Timer,
    cancelled: AtomicBool,
    done_tx: Mutex<Option<Sender<()>>>,
    pending: (Mutex<usize>, Condvar),
    latest: Mutex<HashMap<String, RadState>>,
    finals: Mutex<HashMap<String, Category>>,
    value_mem: Mutex<HashMap<String, Option<(String, String)>>>,
    waiting: [Once; 4],
}

impl Shared {
    /// Builds the shared state and starts the timer and the 4 stages.
    fn start(cfg: &Config, deps: &Deps, items: &[Item]) -> (Arc<Shared>, Vec<JoinHandle<()>>) {
        let (mut send_tx, mut send_rx, mut confirm_tx, mut confirm_rx) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for _ in Step::ALL {
            let (tx, rx) = unbounded();
            send_tx.push(tx);
            send_rx.push(rx);
            let (tx, rx) = unbounded();
            confirm_tx.push(tx);
            confirm_rx.push(rx);
        }
        let (done_tx, done_rx) = bounded::<()>(0);
        let shared = Arc::new(Shared {
            cfg: cfg.clone(),
            api: deps.api.clone(),
            states: deps.states.clone(),
            files: deps.files.clone(),
            values: deps.values.clone(),
            tm: deps.timings.clone(),
            now: deps.now.clone(),
            hook: deps.on_uploaded.clone(),
            progress: deps.on_progress.clone(),
            send_tx,
            confirm_tx,
            timer: Timer::default(),
            cancelled: AtomicBool::new(false),
            done_tx: Mutex::new(Some(done_tx)),
            pending: (Mutex::new(items.len()), Condvar::new()),
            latest: Mutex::new(HashMap::new()),
            finals: Mutex::new(HashMap::new()),
            value_mem: Mutex::new(HashMap::new()),
            waiting: [Once::new(), Once::new(), Once::new(), Once::new()],
        });
        let mut workers = Vec::new();
        let timer_shared = shared.clone();
        workers.push(std::thread::spawn(move || {
            let shared = timer_shared;
            shared.timer.run(&shared);
        }));
        for step in Step::ALL {
            let i = idx(step);
            workers.extend(shared.start_stage(
                step,
                send_rx[i].clone(),
                confirm_rx[i].clone(),
                &done_rx,
            ));
        }
        (shared, workers)
    }

    fn start_stage(
        self: &Arc<Self>,
        step: Step,
        send_rx: Receiver<Item>,
        confirm_rx: Receiver<Item>,
        done: &Receiver<()>,
    ) -> Vec<JoinHandle<()>> {
        let mut handles = Vec::new();
        let (batch_tx, batch_rx) = bounded::<Vec<Item>>(0);
        {
            let (shared, done) = (self.clone(), done.clone());
            handles.push(std::thread::spawn(move || {
                shared.collect(&send_rx, &batch_tx, &done);
            }));
        }
        for _ in 0..self.cfg.send_workers {
            let (shared, done, batch_rx) = (self.clone(), done.clone(), batch_rx.clone());
            handles.push(std::thread::spawn(move || {
                loop {
                    select! {
                        recv(done) -> _ => return,
                        recv(batch_rx) -> batch => match batch {
                            Ok(batch) => shared.send_batch(step, batch),
                            Err(_) => return,
                        },
                    }
                }
            }));
        }
        for _ in 0..self.cfg.confirm_workers {
            let (shared, done, confirm_rx) = (self.clone(), done.clone(), confirm_rx.clone());
            handles.push(std::thread::spawn(move || loop {
                select! {
                    recv(done) -> _ => return,
                    recv(confirm_rx) -> item => match item {
                        Ok(item) => {
                            if let Some(queued) = item.queued_at {
                                shared.tm.observe_queue(queue::CONFIRM, shared.tm.since(queued));
                            }
                            shared.confirm(item);
                        }
                        Err(_) => return,
                    },
                }
            }));
        }
        handles
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Stops the run: workers exit at their next wait, the timer drops what
    /// it still holds.
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        drop(lock(&self.done_tx).take());
        self.timer.close();
    }

    /// Waits until every item left the pipeline, the deadline passes (returns
    /// true) or the process is asked to stop.
    fn wait_until_done(&self, deadline: Duration) -> bool {
        let limit = Instant::now() + deadline;
        let (count, changed) = &self.pending;
        let mut left = lock(count);
        loop {
            if *left == 0 {
                return false;
            }
            if crate::shutdown::requested() {
                return false;
            }
            let now = Instant::now();
            if now >= limit {
                return true;
            }
            let wait = (limit - now).min(Duration::from_millis(100));
            left = changed
                .wait_timeout(left, wait)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    fn now(&self) -> DateTime<Utc> {
        (self.now)()
    }

    fn sender(&self, queue: Queue) -> &Sender<Item> {
        match queue {
            Queue::Send(step) => &self.send_tx[idx(step)],
            Queue::Confirm(step) => &self.confirm_tx[idx(step)],
        }
    }

    /// Puts a due item into its queue (unless the run was cancelled).
    fn deliver(&self, queue: Queue, mut item: Item) {
        if self.is_cancelled() {
            return;
        }
        item.queued_at = Some(self.tm.now());
        let _ = self.sender(queue).send(item);
    }

    /// Queues `item` after `delay`; `kind` names the wait for the timings (the
    /// scheduled delay is recorded, not how long the timer really took).
    fn push(&self, queue: Queue, mut item: Item, delay: Duration, kind: &str) {
        if delay.is_zero() {
            item.queued_at = Some(self.tm.now());
            let _ = self.sender(queue).send(item);
            return;
        }
        self.tm.observe_sleep(kind, delay);
        self.timer.schedule(delay, queue, item);
    }

    /// Time until `at` (zero when it is past or unset).
    fn until(&self, at: Option<DateTime<Utc>>) -> Duration {
        at.and_then(|t| (t - self.now()).to_std().ok())
            .unwrap_or(Duration::ZERO)
    }

    fn dispatch(&self, item: Item) {
        self.report(&item.st.invoice, item.st.step, StepProgress::Enter);
        match item.st.status {
            RadStatus::Sent => {
                let step = item.st.step;
                self.push(
                    Queue::Confirm(step),
                    item,
                    Duration::ZERO,
                    sleep::RESUME_DELAY,
                );
            }
            _ => {
                let (step, delay) = (item.st.step, self.until(item.st.next_attempt_at));
                self.push(Queue::Send(step), item, delay, sleep::RESUME_DELAY);
            }
        }
    }

    /// Logs, once per step and run, that uploads are waiting for Mutual Ser
    /// (the quiet part of a run).
    fn announce_waiting(&self, step: Step, pending: usize) {
        self.waiting[idx(step)].call_once(|| {
            tracing::info!(
                step = step.as_i16(),
                pending,
                "waiting for Mutual Ser confirmation"
            );
        });
    }

    /// Persists the state of `item` (errors are logged, never fatal).
    fn save(&self, item: &mut Item) {
        item.st.updated_at = Some(self.now());
        let t0 = self.tm.now();
        let result = self.states.save(&item.st);
        self.tm.observe_db(self.tm.since(t0));
        if let Err(err) = result {
            tracing::error!(invoice = %item.st.invoice, error = %err, "saving pipeline state");
        }
        lock(&self.latest).insert(item.st.invoice.clone(), item.st.clone());
    }

    /// The item leaves the pipeline in category `cat`.
    fn finish(&self, mut item: Item, cat: Category) {
        self.save(&mut item);
        if matches!(cat, Category::Rejected | Category::Blocked) {
            self.report(&item.st.invoice, item.st.step, StepProgress::Failed);
        }
        lock(&self.finals).insert(item.st.invoice.clone(), cat);
        tracing::info!(
            invoice = %item.st.invoice,
            step = item.st.step.as_i16(),
            status = %item.st.status,
            "invoice left the pipeline"
        );
        let (count, changed) = &self.pending;
        let mut left = lock(count);
        *left = left.saturating_sub(1);
        changed.notify_all();
    }

    /// Groups ready items into micro-batches.
    fn collect(&self, send_rx: &Receiver<Item>, out: &Sender<Vec<Item>>, done: &Receiver<()>) {
        loop {
            let first = select! {
                recv(done) -> _ => return,
                recv(send_rx) -> item => match item {
                    Ok(item) => item,
                    Err(_) => return,
                },
            };
            let mut batch = vec![first];
            let fill_started = self.tm.now();
            let timeout = crossbeam_channel::after(self.cfg.batch_wait);
            while batch.len() < self.cfg.batch_max {
                select! {
                    recv(done) -> _ => return,
                    recv(send_rx) -> item => match item {
                        Ok(item) => batch.push(item),
                        Err(_) => return,
                    },
                    recv(timeout) -> _ => break,
                }
            }
            self.tm
                .observe_sleep(sleep::BATCH_WAIT, self.tm.since(fill_started));
            let ready = self.tm.now();
            select! {
                recv(done) -> _ => return,
                send(out, batch) -> sent => {
                    if sent.is_err() {
                        return;
                    }
                    self.tm.observe_queue(queue::SEND, self.tm.since(ready));
                }
            }
        }
    }

    fn send_batch(&self, step: Step, batch: Vec<Item>) {
        let prepare_started = self.tm.now();
        let ready = self.prepare_batch(batch);
        let prepare = self.tm.since(prepare_started);
        let mut send = Duration::ZERO;
        let files = ready.len();
        self.send_ready(step, ready, &mut send);
        self.tm
            .step_batch(step.as_i16() as usize, files, prepare, send);
    }

    fn send_ready(&self, step: Step, mut ready: Vec<Item>, send: &mut Duration) {
        if ready.is_empty() {
            return;
        }
        let files: Vec<UploadFile> = ready.iter().filter_map(|it| it.file.clone()).collect();
        let (result, took) = self.upload_timed(step, &files, send);
        match result {
            Ok(id) => {
                let uploaded_at = self.tm.now();
                self.announce_waiting(step, ready.len());
                let size = ready.len();
                for item in ready {
                    self.sent(item, &id, size, uploaded_at);
                }
            }
            Err(_) if self.is_cancelled() => {}
            Err(err) if ready.len() == 1 => {
                if let Some(item) = ready.pop() {
                    self.send_failed(item, &err);
                }
            }
            Err(err) => {
                // One bad file must not keep the others from being filed.
                tracing::warn!(
                    step = step.as_i16(),
                    files = ready.len(),
                    duration_ms = took.as_millis() as u64,
                    error = %err,
                    "batch upload failed; sending individually"
                );
                for item in ready {
                    self.tm.resend(step.as_i16() as usize, "batch_split");
                    self.send_alone(step, item, send);
                }
            }
        }
    }

    fn send_alone(&self, step: Step, item: Item, send: &mut Duration) {
        let files: Vec<UploadFile> = item.file.iter().cloned().collect();
        let (result, _) = self.upload_timed(step, &files, send);
        match result {
            Ok(id) => {
                let uploaded_at = self.tm.now();
                self.announce_waiting(step, 1);
                self.sent(item, &id, 1, uploaded_at);
            }
            Err(_) if self.is_cancelled() => {}
            Err(err) => self.send_failed(item, &err),
        }
    }

    /// Prepares every item of the batch, up to `prepare_concurrency` at a
    /// time, and returns the ones ready to upload in batch order. Once the run
    /// is cancelled no further item is started.
    fn prepare_batch(&self, batch: Vec<Item>) -> Vec<Item> {
        let workers = self.cfg.prepare_concurrency.min(batch.len()).max(1);
        par::map(batch, workers, |item| {
            if self.is_cancelled() {
                return None;
            }
            self.prepare(item)
        })
        .into_iter()
        .flatten()
        .collect()
    }

    fn upload_timed(
        &self,
        step: Step,
        files: &[UploadFile],
        send: &mut Duration,
    ) -> (Result<String, ApiError>, Duration) {
        let started = self.tm.now();
        let size: u64 = files.iter().map(|f| f.size).sum();
        tracing::info!(
            step = step.as_i16(),
            files = files.len(),
            bytes = size,
            "uploading step batch"
        );
        let result = self.api.upload(step, files);
        let took = self.tm.since(started);
        *send += took;
        if let Ok(id) = &result {
            tracing::info!(
                step = step.as_i16(),
                files = files.len(),
                id_cargue = %id,
                duration_ms = took.as_millis() as u64,
                "step batch uploaded"
            );
        }
        (result, took)
    }

    /// `files.find`, timed as local-file work.
    fn find_file(&self, step: Step, invoice: &str, archived: bool) -> Result<Found> {
        let t0 = self.tm.now();
        let found = self.files.find(step, invoice, archived);
        self.tm.observe_fs(self.tm.since(t0));
        found
    }

    /// The local checks before an upload. `None` when the item was resolved
    /// (blocked, rejected or already confirmed) without sending.
    fn prepare(&self, mut item: Item) -> Option<Item> {
        let step = item.st.step;
        if self.cfg.max_sends > 0 && item.st.sends >= self.cfg.max_sends {
            item.st.status = RadStatus::Rejected;
            item.st.last_detail = format!(
                "paso={} Radicacion rechazada: se agotaron {} envios del paso {}.",
                step.as_i16(),
                item.st.sends,
                step.label()
            );
            self.finish(item, Category::Rejected);
            return None;
        }
        if step == Step::InvoiceZip
            && self.values.is_some()
            && let Some((code, detail)) = self.check_value(&item.st.invoice)
        {
            self.block_value(item, &code, &detail);
            return None;
        }
        let skip_precheck = item.advanced_here && item.st.sends == 0;
        if matches!(step, Step::Rips | Step::Crc) && !skip_precheck {
            // Mutual Ser may already have this step (filed by another run or tool).
            match self.precheck_evidence(&item) {
                Err(_) if self.is_cancelled() => return None,
                Err(err) => {
                    tracing::warn!(invoice = %item.st.invoice, step = step.as_i16(), error = %err, "pre-check failed; uploading anyway");
                }
                Ok(ev) => {
                    let verdict = evaluate(step, &ev);
                    if !matches!(verdict.outcome, Outcome::Pending | Outcome::Failed) {
                        self.apply(item, verdict, None);
                        return None;
                    }
                }
            }
        }
        let archived = item.st.reload_for_step2 && step == Step::InvoiceZip;
        let found = match self.find_file(step, &item.st.invoice, archived) {
            Ok(found) => found,
            Err(err) => Found::Missing(err.to_string()),
        };
        match found {
            Found::File(file) => {
                item.file = Some(file);
                Some(item)
            }
            Found::Missing(diag) => {
                self.block_missing(item, &diag);
                None
            }
        }
    }

    /// The step 1 value check, cached per invoice for the run. `None` = may send.
    fn check_value(&self, invoice: &str) -> Option<(String, String)> {
        if let Some(cached) = lock(&self.value_mem).get(invoice) {
            return cached.clone();
        }
        let result = self.compute_value(invoice);
        if !self.is_cancelled() {
            lock(&self.value_mem).insert(invoice.to_owned(), result.clone());
        }
        result
    }

    fn compute_value(&self, invoice: &str) -> Option<(String, String)> {
        let unavailable = |err: &dyn std::fmt::Display| {
            Some((
                super::rules::CODE_VALIDATION_UNAVAILABLE.to_owned(),
                format!("No fue posible comprobar los valores antes de radicar: {err}"),
            ))
        };
        let values = self.values.as_ref()?;
        let zip = match self.find_file(Step::InvoiceZip, invoice, true) {
            Ok(Found::File(file)) => Some(file),
            Ok(Found::Missing(_)) => None,
            Err(err) => return unavailable(&err),
        };
        let from_zip = zip.and_then(|file| {
            let t0 = self.tm.now();
            let total = self.files.xml_total(&file);
            self.tm.observe_fs(self.tm.since(t0));
            total.map(|(amount, source)| (amount, source, file.rel))
        });
        let t0 = self.tm.now();
        let amount = match from_zip {
            Some((amount, source, rel)) => {
                let synced = values.sync_xml_total(invoice, &amount, &source, &rel);
                self.tm.observe_db(self.tm.since(t0));
                if let Err(err) = synced {
                    return unavailable(&err);
                }
                Some(amount)
            }
            None => {
                let stored = values.stored_xml_total(invoice);
                self.tm.observe_db(self.tm.since(t0));
                match stored {
                    Err(err) => return unavailable(&err),
                    Ok(stored) => stored.map(|(amount, _)| amount),
                }
            }
        };
        let agreement = match &amount {
            None => None,
            Some(_) => {
                let t0 = self.tm.now();
                let total = values.agreement_total(invoice);
                self.tm.observe_db(self.tm.since(t0));
                match total {
                    Err(err) => return unavailable(&err),
                    Ok(total) => total,
                }
            }
        };
        check_value(
            amount.as_deref().unwrap_or(""),
            amount.is_some(),
            agreement.as_deref().unwrap_or(""),
            agreement.is_some(),
        )
        .map(|(code, detail)| (code.to_owned(), detail))
    }

    fn block_value(&self, mut item: Item, code: &str, detail: &str) {
        tracing::warn!(invoice = %item.st.invoice, code, "step 1 blocked by value validation");
        item.st.status = RadStatus::Blocked;
        item.st.last_detail =
            format!("codigo={code} mensaje={detail} paso1_no_enviado=true pasos_omitidos=2,3,4");
        self.finish(item, Category::Blocked);
    }

    fn block_missing(&self, mut item: Item, diag: &str) {
        let (invoice, step) = (item.st.invoice.clone(), item.st.step);
        item.st.last_detail = if step == Step::Cuv {
            format!(
                "Radicacion detenida despues del paso 1: factura {invoice} no tiene TXT con CUV exitoso. {diag}"
            )
        } else {
            format!(
                "No se radica {} porque no se encontro archivo para factura {invoice}. {diag}",
                step.label()
            )
        };
        tracing::warn!(invoice = %invoice, step = step.as_i16(), diag, "step file not found");
        item.st.status = RadStatus::Blocked;
        self.finish(item, Category::Blocked);
    }

    /// The batch was uploaded: record it and schedule the first poll.
    fn sent(&self, mut item: Item, id: &str, batch_size: usize, uploaded_at: Instant) {
        let step = item.st.step;
        item.sent_tick = Some(uploaded_at);
        item.send_errors = 0;
        item.st.status = RadStatus::Sent;
        item.st.upload_id = id.to_owned();
        item.st.sent_at = Some(self.now());
        item.st.polls = 0;
        item.st.last_detail = format!(
            "paso={} Enviado por lote {} idCargue={id} tamanoLote={batch_size}",
            step.as_i16(),
            step.label()
        );
        item.st.sends += 1;
        self.save(&mut item);
        tracing::info!(invoice = %item.st.invoice, step = step.as_i16(), id_cargue = %id, batch = batch_size, "step uploaded");
        let invoice = item.st.invoice.clone();
        self.push(
            Queue::Confirm(step),
            item,
            self.cfg.first_poll_delay,
            sleep::FIRST_POLL,
        );
        self.notify_uploaded(&invoice, step);
    }

    /// Reports a step transition to the dashboard sink; it can never break the run.
    fn report(&self, invoice: &str, step: Step, what: StepProgress) {
        let Some(sink) = &self.progress else { return };
        let call = std::panic::AssertUnwindSafe(|| sink(invoice, step, what));
        if std::panic::catch_unwind(call).is_err() {
            tracing::error!(invoice, step = step.as_i16(), "progress sink panicked");
        }
    }

    /// Calls the optional hook; it can never break the run.
    fn notify_uploaded(&self, invoice: &str, step: Step) {
        let Some(hook) = &self.hook else { return };
        let call = std::panic::AssertUnwindSafe(|| hook(invoice, step));
        if std::panic::catch_unwind(call).is_err() {
            tracing::error!(invoice, step = step.as_i16(), "on_uploaded hook panicked");
        }
    }

    fn send_failed(&self, mut item: Item, err: &ApiError) {
        let step = item.st.step;
        self.tm.resend(step.as_i16() as usize, "send_error");
        item.send_errors += 1;
        item.st.sends += 1;
        let delay = backoff(
            self.cfg.quick_retry_delay,
            item.send_errors,
            self.cfg.retry_interval,
        );
        item.st.status = RadStatus::Pending;
        item.st.last_detail = format!(
            "paso={} Fallo el envio {}: {err}",
            step.as_i16(),
            step.label()
        );
        item.st.next_attempt_at = Some(self.now() + chrono_delay(delay));
        self.save(&mut item);
        tracing::warn!(invoice = %item.st.invoice, step = step.as_i16(), retry_in_s = delay.as_secs(), error = %err, "upload failed");
        self.push(Queue::Send(step), item, delay, sleep::SEND_RETRY);
    }

    /// The upload date for `findLoad` (30 days back when the row has no send time).
    fn since(&self, item: &Item) -> DateTime<Utc> {
        item.st
            .sent_at
            .unwrap_or_else(|| self.now() - chrono::Duration::days(30))
    }

    /// The prepare-time portal query, capped by `precheck_timeout`. The cap
    /// lives on a helper thread only: when it expires the run goes on and the
    /// item is uploaded, like on any other pre-check error.
    fn precheck_evidence(&self, item: &Item) -> Result<Evidence, String> {
        let query = EvidenceQuery::new(self, item, false);
        let Some(cap) = self.cfg.precheck_cap() else {
            return query.gather(self.api.as_ref()).map_err(|e| e.to_string());
        };
        let (tx, rx) = bounded(1);
        let api = self.api.clone();
        std::thread::spawn(move || {
            let _ = tx.send(query.gather(api.as_ref()).map_err(|e| e.to_string()));
        });
        rx.recv_timeout(cap)
            .unwrap_or_else(|_| Err(format!("la consulta previa superó {}s", cap.as_secs_f32())))
    }

    /// What Mutual Ser knows about the item's current step.
    fn evidence(&self, item: &Item) -> Result<Evidence, ApiError> {
        EvidenceQuery::new(self, item, true).gather(self.api.as_ref())
    }

    fn confirm(&self, item: Item) {
        match self.evidence(&item) {
            Err(err) => {
                if self.is_cancelled() {
                    return;
                }
                tracing::warn!(invoice = %item.st.invoice, step = item.st.step.as_i16(), error = %err, "confirmation query failed");
                let detail = item.st.last_detail.clone();
                self.pending_poll(item, Verdict::pending(&detail), None);
            }
            Ok(ev) => {
                let verdict = evaluate(item.st.step, &ev);
                self.apply(item, verdict, ev.load);
            }
        }
    }

    /// Moves the item according to a verdict.
    fn apply(&self, item: Item, verdict: Verdict, load: Option<Load>) {
        match verdict.outcome {
            Outcome::Confirmed => self.confirmed(item, &verdict),
            Outcome::Pending => self.pending_poll(item, verdict, load),
            Outcome::Failed => self.failed(item, &verdict),
            Outcome::AdjustmentNote => self.leave(item, RadStatus::AdjustmentNote, &verdict),
            Outcome::CreditNote => self.leave(item, RadStatus::CreditNote, &verdict),
        }
    }

    fn leave(&self, mut item: Item, status: RadStatus, verdict: &Verdict) {
        item.st.status = status;
        item.st.last_detail = verdict.detail.clone();
        self.finish(item, Category::Rejected);
    }

    fn pending_poll(&self, mut item: Item, mut verdict: Verdict, load: Option<Load>) {
        let step = item.st.step;
        item.st.polls += 1;
        if !verdict.detail.is_empty() {
            item.st.last_detail = verdict.detail.clone();
        }
        if step == Step::Cuv && verdict.file_loaded && item.st.polls >= self.cfg.step2_xml_polls {
            verdict.outcome = Outcome::Failed;
            verdict.message = "filter-xml no muestra la factura ACTIVE".to_owned();
            self.failed(item, &verdict);
            return;
        }
        let crc_lost = step == Step::Crc
            && !verdict.file_loaded
            && load
                .as_ref()
                .is_some_and(|l| upload_lost(&l.raw, self.now(), self.cfg.crc_lost_after));
        if crc_lost {
            tracing::warn!(invoice = %item.st.invoice, id_cargue = %item.st.upload_id, "step 4 upload lost; re-sending");
            item.st.status = RadStatus::Pending;
            item.st.upload_id.clear();
            item.sent_tick = None;
            self.tm.resend(step.as_i16() as usize, "crc_lost");
            self.save(&mut item);
            self.push(Queue::Send(step), item, Duration::ZERO, sleep::POLL);
            return;
        }
        let delay = backoff(
            self.cfg.poll_interval,
            item.st.polls as u32,
            self.cfg.poll_max_interval,
        );
        item.st.next_attempt_at = Some(self.now() + chrono_delay(delay));
        self.save(&mut item);
        let queue = if item.st.status == RadStatus::Sent {
            Queue::Confirm(step)
        } else {
            Queue::Send(step)
        };
        self.push(queue, item, delay, sleep::POLL);
    }

    fn failed(&self, mut item: Item, verdict: &Verdict) {
        let (step, invoice) = (item.st.step, item.st.invoice.clone());
        let text = format!("{} {}", verdict.detail, verdict.message);
        if step == Step::Cuv
            && requires_step1_reload(&text)
            && !item.st.step1_reloaded
            && self.cfg.only_step.is_none()
        {
            tracing::warn!(invoice = %invoice, detail = %verdict.message, "step 2 needs step 1 again");
            item.st.step1_reloaded = true;
            item.st.reload_for_step2 = true;
            self.report(&invoice, step, StepProgress::Back);
            self.report(&invoice, Step::InvoiceZip, StepProgress::Enter);
            item.st.step = Step::InvoiceZip;
            item.st.status = RadStatus::Pending;
            item.st.upload_id.clear();
            item.st.sends = 0;
            item.st.polls = 0;
            item.st.last_detail = verdict.detail.clone();
            item.file = None;
            item.sent_tick = None;
            self.tm.resend(step.as_i16() as usize, "step2_reload");
            self.save(&mut item);
            self.push(
                Queue::Send(Step::InvoiceZip),
                item,
                Duration::ZERO,
                sleep::RESEND,
            );
            return;
        }
        let quick = retryable_upload_message(step, &verdict.message)
            && item.st.sends < self.cfg.quick_retries;
        let (delay, reason) = if quick {
            (self.cfg.quick_retry_delay, "quick_retry")
        } else {
            (self.cfg.retry_interval, "retry_interval")
        };
        item.sent_tick = None;
        self.tm.resend(step.as_i16() as usize, reason);
        item.st.status = RadStatus::Pending;
        item.st.upload_id.clear();
        item.st.last_detail = verdict.detail.clone();
        item.st.next_attempt_at = Some(self.now() + chrono_delay(delay));
        self.save(&mut item);
        tracing::warn!(invoice = %invoice, step = step.as_i16(), retry_in_s = delay.as_secs(), message = %verdict.message, "step failed; will re-send");
        self.push(Queue::Send(step), item, delay, sleep::RESEND);
    }

    fn confirmed(&self, mut item: Item, verdict: &Verdict) {
        let step = item.st.step;
        if let Some(sent) = item.sent_tick.take() {
            // Only uploads made (and confirmed) in this run.
            self.tm.confirmed(
                step.as_i16() as usize,
                self.tm.since(sent),
                item.st.polls as u64 + 1,
            );
        }
        self.archive(&item);
        tracing::info!(invoice = %item.st.invoice, step = step.as_i16(), id_cargue = %item.st.upload_id, "step confirmed");
        item.st.last_detail = verdict.detail.clone();
        if step == Step::InvoiceZip && item.st.reload_for_step2 {
            item.st.reload_for_step2 = false;
            self.report(&item.st.invoice, step, StepProgress::Back);
            self.advance(item, self.cfg.reload_delay());
            return;
        }
        self.report(&item.st.invoice, step, StepProgress::Done);
        self.advance(item, Duration::ZERO);
    }

    /// Marks the filed file as archived.
    fn archive(&self, item: &Item) {
        let file = match &item.file {
            Some(file) => file.clone(),
            None => match self.find_file(item.st.step, &item.st.invoice, false) {
                Ok(Found::File(file)) => file,
                _ => return,
            },
        };
        let t0 = self.tm.now();
        let result = self.files.archive(&file);
        self.tm.observe_fs(self.tm.since(t0));
        if let Err(err) = result {
            tracing::error!(invoice = %item.st.invoice, file = %file.rel, error = %err, "archiving filed file");
        }
    }

    fn advance(&self, mut item: Item, delay: Duration) {
        let Some(next) = item.st.step.next() else {
            item.st.status = RadStatus::Confirmed;
            self.finish(item, Category::Done);
            return;
        };
        item.st.step = next;
        item.st.status = RadStatus::Pending;
        item.st.upload_id.clear();
        item.st.sent_at = None;
        item.st.sends = 0;
        item.st.polls = 0;
        item.st.next_attempt_at = None;
        item.file = None;
        item.send_errors = 0;
        item.sent_tick = None;
        item.advanced_here = true;
        if self.cfg.only_step.is_some() {
            self.finish(item, Category::Advanced);
            return;
        }
        self.report(&item.st.invoice, next, StepProgress::Enter);
        self.save(&mut item);
        self.push(Queue::Send(next), item, delay, sleep::RELOAD);
    }

    /// Fills `report` from what the run recorded.
    fn fill_report(&self, report: &mut Report, done: &[RadState], skipped: &[RadState]) {
        for st in done {
            report.done.push(st.invoice.clone());
            report.states.insert(st.invoice.clone(), st.clone());
            report.skipped.push(st.invoice.clone());
        }
        for st in skipped {
            report.states.insert(st.invoice.clone(), st.clone());
            report.skipped.push(st.invoice.clone());
        }
        let latest = lock(&self.latest).clone();
        let finals = lock(&self.finals).clone();
        let mut invoices: Vec<&String> = latest.keys().collect();
        invoices.sort();
        for invoice in invoices {
            let st = &latest[invoice];
            report.states.insert(invoice.clone(), st.clone());
            match finals.get(invoice) {
                None if st.status == RadStatus::Sent || still_processing(&st.last_detail) => {
                    report.in_progress.push(invoice.clone());
                }
                None => report.unclassified.push(invoice.clone()),
                Some(Category::Done) => report.done.push(invoice.clone()),
                Some(Category::Advanced) => report.advanced.push(invoice.clone()),
                Some(Category::Rejected) => report.rejected.push(invoice.clone()),
                Some(Category::Blocked) => report.blocked.push(invoice.clone()),
            }
        }
    }
}

/// What to ask Mutual Ser about one item's current step.
struct EvidenceQuery {
    step: Step,
    invoice: String,
    nit: String,
    upload_id: String,
    since: DateTime<Utc>,
    with_load: bool,
}

impl EvidenceQuery {
    fn new(shared: &Shared, item: &Item, with_load: bool) -> EvidenceQuery {
        EvidenceQuery {
            step: item.st.step,
            invoice: item.st.invoice.clone(),
            nit: shared.cfg.nit.clone(),
            upload_id: item.st.upload_id.clone(),
            since: shared.since(item),
            with_load,
        }
    }

    /// Gathers the evidence of the step (Go `evidenceCtx`).
    fn gather(&self, api: &dyn RadApi) -> Result<Evidence, ApiError> {
        let mut ev = Evidence {
            invoice: self.invoice.clone(),
            nit: self.nit.clone(),
            ..Evidence::default()
        };
        if self.with_load && !self.upload_id.is_empty() {
            ev.id_cargue = self.upload_id.clone();
            ev.load = Some(api.find_load(&self.upload_id, Some(self.since))?);
        }
        match self.step {
            Step::InvoiceZip => {}
            Step::Cuv => {
                let verdict = evaluate(self.step, &ev);
                if verdict.outcome == Outcome::Pending && verdict.file_loaded {
                    ev.xml = api.filter_xml(&self.invoice)?;
                }
            }
            Step::Rips | Step::Crc => {
                ev.filters = api.filters_all(&self.invoice)?;
                let file_id = ev
                    .filters
                    .as_ref()
                    .map(|f| f.file_id.clone())
                    .unwrap_or_default();
                if self.step == Step::Rips
                    && !file_id.is_empty()
                    && evaluate(self.step, &ev).outcome == Outcome::AdjustmentNote
                {
                    match api.file_details(&file_id) {
                        Ok(details) => ev.details = details,
                        Err(err) => {
                            tracing::warn!(invoice = %self.invoice, error = %err, "findDetailsByFile failed");
                        }
                    }
                }
            }
        }
        Ok(ev)
    }
}

/// `base` doubled `n - 1` times, capped at `ceiling`.
fn backoff(base: Duration, n: u32, ceiling: Duration) -> Duration {
    let mut delay = base;
    for _ in 1..n {
        if delay >= ceiling {
            break;
        }
        delay = delay.saturating_mul(2);
    }
    delay.min(ceiling)
}

fn chrono_delay(delay: Duration) -> chrono::Duration {
    chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::MAX)
}
