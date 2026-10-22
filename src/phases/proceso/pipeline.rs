//! The streaming engine of `proceso run` (Go `runPipeline`).
//!
//! Every active phase is a pool of worker threads fed by its own queue; an
//! invoice advances to the next phase the moment it clears the current one,
//! so a slow invoice never holds back the others. Each phase's concurrency is
//! adaptive ([`PhaseConcurrency`]). Radicación is the only batch gate: cleared
//! invoices are queued and filed in batches of `threshold`, or sooner when the
//! oldest queued invoice has waited `interval`; uploads are serialized and the
//! last partial batch is flushed at the end.
//!
//! With a supplier and a target, an invoice that leaves the per-invoice
//! phases without clearing the last one is replaced by a fresh invoice (see
//! [`Hooks::supply`]) until `target` invoices cleared, the pool is spent, the
//! supplier fails or `breaker` consecutive drops pause replacement.
//!
//! The engine knows nothing about databases or phases: the caller passes
//! closures ([`Hooks`]), which is what makes it testable with fakes.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, unbounded};

use super::classify::{Status, Verdict};
use super::concurrency::PhaseConcurrency;
use super::events::{Event, Events};

/// Consecutive drops without a clear that pause replacement (Go 10).
pub const DEFAULT_BREAKER: usize = 10;

/// One per-invoice stage.
#[derive(Debug, Clone)]
pub struct PhaseSpec {
    pub name: &'static str,
    pub label: &'static str,
    /// Worker threads and ceiling of the adaptive concurrency.
    pub workers: usize,
    /// Default successor; `None` = the invoice cleared the last phase.
    pub next: Option<&'static str>,
}

/// The radicación batch gate.
#[derive(Debug, Clone)]
pub struct GateConfig {
    pub threshold: usize,
    /// Longest the oldest queued invoice waits (zero disables the timer).
    pub interval: Duration,
    pub label: &'static str,
}

/// What filing one batch ended as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fired {
    /// Filed; the text is an optional summary for the event.
    Done(String),
    /// Another machine is filing this cargue; not a failure of this run.
    OtherMachine(String),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub phases: Vec<PhaseSpec>,
    /// First phase; `None` sends every invoice straight to the gate.
    pub entry: Option<&'static str>,
    pub invoices: Vec<String>,
    pub continue_on_error: bool,
    pub gate: Option<GateConfig>,
    /// Invoices that must clear the last phase (0 = no replacement).
    pub target: usize,
    /// How many replacements can ever be injected.
    pub replace_cap: usize,
    pub breaker: usize,
}

/// Runs one phase for one invoice.
pub type RunFn<'a> = &'a (dyn Fn(&str, &str) -> Verdict + Sync);
/// Files one batch.
pub type FireFn<'a> = &'a (dyn Fn(&[String]) -> Fired + Sync);
/// Returns up to `n` new, prepared invoices; fewer means the pool is spent.
pub type SupplyFn<'a> = &'a (dyn Fn(usize) -> Result<Vec<String>> + Sync);

pub struct Hooks<'a> {
    pub run: RunFn<'a>,
    pub fire: Option<FireFn<'a>>,
    pub supply: Option<SupplyFn<'a>>,
}

/// Final tallies of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub worst: Status,
    /// The first systemic error.
    pub error: Option<String>,
    pub done: usize,
    pub manual: usize,
    pub excluded: usize,
    pub failed: usize,
    pub skipped: usize,
    /// Invoices that entered, initial plus replacements.
    pub entered: usize,
    pub replaced: usize,
}

struct Inflight {
    count: Mutex<usize>,
    zero: Condvar,
}

impl Inflight {
    fn new() -> Self {
        Self {
            count: Mutex::new(0),
            zero: Condvar::new(),
        }
    }

    fn add(&self, n: usize) {
        *lock(&self.count) += n;
    }

    fn done(&self) {
        let mut count = lock(&self.count);
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.zero.notify_all();
        }
    }

    fn wait(&self) {
        let mut count = lock(&self.count);
        while *count > 0 {
            count = self.zero.wait(count).unwrap_or_else(|e| e.into_inner());
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
struct Counts {
    done: usize,
    manual: usize,
    excluded: usize,
    failed: usize,
    skipped: usize,
}

struct State {
    error: Option<String>,
    worst: Status,
    counts: Counts,
    phase_ms: BTreeMap<String, u64>,
    entered: usize,
    dropped: usize,
    injected: usize,
    drop_streak: usize,
    /// Drops without a replacement yet, so each injected invoice names the
    /// drop it covers.
    pending_drops: VecDeque<(String, String)>,
}

#[derive(Default)]
struct GateState {
    batch: Vec<String>,
    first_at: Option<Instant>,
    closed: bool,
}

struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

impl Gate {
    /// Removes the pending batch; the caller holds the lock.
    fn take(&self, state: &mut GateState) -> Vec<String> {
        state.first_at = None;
        self.changed.notify_all();
        std::mem::take(&mut state.batch)
    }
}

enum Next {
    Cleared,
    DropManual,
    Phase(&'static str),
}

struct Run<'a> {
    cfg: &'a PipelineConfig,
    hooks: &'a Hooks<'a>,
    events: &'a Events,
    cancelled: AtomicBool,
    inflight: Inflight,
    senders: Mutex<HashMap<&'static str, Sender<String>>>,
    state: Mutex<State>,
    gate: Gate,
    fire_lock: Mutex<()>,
    /// Serializes "compute need + supply + inject"; true once replacement is over.
    supply_stopped: Mutex<bool>,
    replace_on: bool,
    replace_cap: usize,
    breaker: usize,
}

/// Streams `cfg.invoices` through the phases and returns the tallies.
pub fn run_pipeline(cfg: &PipelineConfig, hooks: &Hooks<'_>, events: &Events) -> Finished {
    let replace_on = cfg.target > 0 && hooks.supply.is_some();
    let run = Run {
        cfg,
        hooks,
        events,
        cancelled: AtomicBool::new(false),
        inflight: Inflight::new(),
        senders: Mutex::new(HashMap::new()),
        state: Mutex::new(State {
            error: None,
            worst: Status::Ok,
            counts: Counts::default(),
            phase_ms: BTreeMap::new(),
            entered: cfg.invoices.len(),
            dropped: 0,
            injected: 0,
            drop_streak: 0,
            pending_drops: VecDeque::new(),
        }),
        gate: Gate {
            state: Mutex::new(GateState::default()),
            changed: Condvar::new(),
        },
        fire_lock: Mutex::new(()),
        supply_stopped: Mutex::new(false),
        replace_on,
        replace_cap: if replace_on { cfg.replace_cap } else { 0 },
        breaker: if cfg.breaker == 0 {
            DEFAULT_BREAKER
        } else {
            cfg.breaker
        },
    };
    run.execute()
}

impl Run<'_> {
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst) || crate::shutdown::requested()
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.gate.changed.notify_all();
    }

    fn execute(&self) -> Finished {
        let mut start = Event::new("run_start");
        start.total = self.cfg.invoices.len();
        self.events.emit(start);

        let mut receivers: Vec<(&PhaseSpec, Receiver<String>, PhaseConcurrency)> = Vec::new();
        for spec in &self.cfg.phases {
            let (tx, rx) = unbounded();
            lock(&self.senders).insert(spec.name, tx);
            receivers.push((spec, rx, PhaseConcurrency::new(spec.workers)));
        }
        let timed = self
            .cfg
            .gate
            .as_ref()
            .is_some_and(|g| !g.interval.is_zero())
            && self.hooks.fire.is_some();
        std::thread::scope(|scope| {
            for (spec, rx, concurrency) in &receivers {
                for _ in 0..spec.workers.max(1) {
                    scope.spawn(move || self.worker(spec, rx, concurrency));
                }
            }
            if timed {
                scope.spawn(|| self.timer());
            }
            self.inflight.add(self.cfg.invoices.len());
            for invoice in &self.cfg.invoices {
                self.enter(invoice.clone());
            }
            self.inflight.wait();
            lock(&self.senders).clear();
            lock(&self.gate.state).closed = true;
            self.gate.changed.notify_all();
        });
        // The timer is gone: flush the last, partial batch.
        let last = {
            let mut gate = lock(&self.gate.state);
            self.gate.take(&mut gate)
        };
        if !self.cancelled() {
            self.fire_batch(last);
        }
        self.finish()
    }

    fn finish(&self) -> Finished {
        let state = lock(&self.state);
        let mut end = Event::new("run_end").status(state.worst.as_str());
        end.done = state.counts.done;
        end.manual = state.counts.manual;
        end.excluded = state.counts.excluded;
        end.failed = state.counts.failed;
        end.skipped = state.counts.skipped;
        end.phase_ms = state.phase_ms.clone();
        if self.replace_on {
            end.total = state.entered;
            end.replaced = state.injected;
        }
        self.events.emit(end);
        Finished {
            worst: state.worst,
            error: state.error.clone(),
            done: state.counts.done,
            manual: state.counts.manual,
            excluded: state.counts.excluded,
            failed: state.counts.failed,
            skipped: state.counts.skipped,
            entered: state.entered,
            replaced: state.injected,
        }
    }

    fn worker(&self, spec: &PhaseSpec, queue: &Receiver<String>, concurrency: &PhaseConcurrency) {
        for invoice in queue.iter() {
            let stamp = concurrency.acquire();
            let conflict = self.process(spec, &invoice);
            concurrency.release(stamp, conflict);
        }
    }

    /// Sends a new invoice to the entry phase (or straight to the gate).
    fn enter(&self, invoice: String) {
        match self.cfg.entry {
            Some(entry) => self.send(entry, invoice),
            None => {
                if !self.cancelled() {
                    self.to_gate(&invoice);
                }
                self.inflight.done();
            }
        }
    }

    /// Queues an invoice for a phase; a cancelled run drops it instead.
    fn send(&self, phase: &'static str, invoice: String) {
        if self.cancelled() {
            self.inflight.done();
            return;
        }
        let sender = lock(&self.senders).get(phase).cloned();
        let sent = sender.is_some_and(|tx| tx.send(invoice).is_ok());
        if !sent {
            self.inflight.done();
        }
    }

    fn resolve_next(&self, spec: &PhaseSpec, route: Option<&'static str>) -> Next {
        match route {
            None => spec.next.map_or(Next::Cleared, Next::Phase),
            Some(name) => match self.cfg.phases.iter().find(|p| p.name == name) {
                Some(found) => Next::Phase(found.name),
                // The routed phase is not active (fev flags a value
                // difference but conciliar is skipped).
                None => Next::DropManual,
            },
        }
    }

    /// The whole per-invoice unit of work; returns whether a transient
    /// database conflict was seen (it halves the phase's concurrency).
    fn process(&self, spec: &PhaseSpec, invoice: &str) -> bool {
        if self.cancelled() {
            self.inflight.done();
            return false;
        }
        self.events.emit(
            Event::new("invoice_enter")
                .invoice(invoice)
                .phase(spec.name, spec.label),
        );
        let started = Instant::now();
        let verdict = catch_unwind(AssertUnwindSafe(|| (self.hooks.run)(spec.name, invoice)))
            .unwrap_or_else(|_| Verdict {
                error: Some(format!("la fase {} terminó con un pánico", spec.name)),
                ..Verdict::new(Status::Failed, "")
            });
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        *lock(&self.state)
            .phase_ms
            .entry(spec.name.to_owned())
            .or_default() += ms;
        self.events.emit(
            Event::new("invoice_step")
                .invoice(invoice)
                .phase(spec.name, spec.label)
                .status(verdict.status.as_str())
                .millis(ms)
                .reason(&verdict.reason)
                .error(verdict.error.as_deref().unwrap_or_default()),
        );
        self.route(spec, invoice, &verdict);
        verdict.conflict
    }

    fn route(&self, spec: &PhaseSpec, invoice: &str, verdict: &Verdict) {
        match verdict.status {
            Status::Ok => match self.resolve_next(spec, verdict.route) {
                Next::Cleared => {
                    lock(&self.state).drop_streak = 0;
                    self.to_gate(invoice);
                    self.inflight.done();
                }
                Next::DropManual => self.drop_invoice(Status::Manual, spec, invoice),
                Next::Phase(next) => self.send(next, invoice.to_owned()),
            },
            Status::Skipped => {
                lock(&self.state).counts.skipped += 1;
                self.inflight.done();
            }
            Status::Manual | Status::Excluded => self.drop_invoice(verdict.status, spec, invoice),
            Status::Failed => self.fail(spec, invoice, verdict),
        }
    }

    fn drop_invoice(&self, status: Status, spec: &PhaseSpec, invoice: &str) {
        {
            let mut state = lock(&self.state);
            count(&mut state, status);
            state.dropped += 1;
            state.drop_streak += 1;
        }
        self.replace_dropped(invoice, spec.name);
        self.inflight.done();
    }

    fn fail(&self, spec: &PhaseSpec, invoice: &str, verdict: &Verdict) {
        let abort;
        {
            let mut state = lock(&self.state);
            count(&mut state, Status::Failed);
            if let Some(error) = &verdict.error
                && state.error.is_none()
            {
                state.error = Some(format!(
                    "phase {:?}, invoice {invoice}: {error}",
                    spec.label
                ));
            }
            abort = verdict.error.is_some()
                && (!(self.cfg.continue_on_error || verdict.resumable) || verdict.fatal);
            state.dropped += 1;
            state.drop_streak += 1;
        }
        if abort {
            self.cancel();
        }
        self.replace_dropped(invoice, spec.name);
        self.inflight.done();
    }

    /// Asks the supplier for replacements after `invoice` dropped in `phase`.
    /// Runs before the dropping worker's own `inflight.done()`, so the run
    /// cannot end while a replacement is still being routed.
    fn replace_dropped(&self, invoice: &str, phase: &str) {
        if !self.replace_on {
            return;
        }
        lock(&self.state)
            .pending_drops
            .push_back((invoice.to_owned(), phase.to_owned()));
        let fresh = self.supply_round(invoice, phase);
        for new in fresh {
            self.enter(new);
        }
    }

    fn supply_round(&self, invoice: &str, phase: &str) -> Vec<String> {
        let mut stopped = lock(&self.supply_stopped);
        if *stopped || self.cancelled() {
            return Vec::new();
        }
        let (streak, viable, remaining) = {
            let state = lock(&self.state);
            (
                state.drop_streak,
                state.entered - state.dropped,
                self.replace_cap.saturating_sub(state.injected),
            )
        };
        if streak >= self.breaker {
            *stopped = true;
            self.events.emit(Event::new("replace_stopped").reason(&format!(
                "{streak} consecutive invoices stopped before radicación without one clearing; replacement paused"
            )));
            return Vec::new();
        }
        let need = self.cfg.target.saturating_sub(viable);
        if need == 0 {
            return Vec::new();
        }
        if remaining == 0 {
            *stopped = true; // pool exhausted
            return Vec::new();
        }
        let need = need.min(remaining);
        let Some(supply) = self.hooks.supply else {
            return Vec::new();
        };
        let mut got = match supply(need) {
            Ok(got) => got,
            Err(err) => {
                *stopped = true;
                if !self.cancelled() {
                    self.events.emit(
                        Event::new("replace_stopped").reason(&format!("supply error: {err:#}")),
                    );
                }
                return Vec::new();
            }
        };
        if got.len() < need {
            *stopped = true; // the pool has no more invoices
        }
        got.truncate(remaining);
        let mut fresh = Vec::new();
        for new in got {
            if self.cancelled() {
                break;
            }
            self.inflight.add(1);
            let cover = {
                let mut state = lock(&self.state);
                state.entered += 1;
                state.injected += 1;
                state
                    .pending_drops
                    .pop_front()
                    .unwrap_or_else(|| (invoice.to_owned(), phase.to_owned()))
            };
            self.events.emit(
                Event::new("invoice_replace")
                    .invoice(&new)
                    .phase(&cover.1, "")
                    .reason(&format!("replaces {}", cover.0)),
            );
            fresh.push(new);
        }
        fresh
    }

    /// Adds a cleared invoice to the radicación batch and files a full batch
    /// at once; the first invoice of a batch arms the interval timer.
    fn to_gate(&self, invoice: &str) {
        let (Some(gate), Some(_)) = (&self.cfg.gate, self.hooks.fire) else {
            lock(&self.state).counts.done += 1;
            return;
        };
        let ready = {
            let mut state = lock(&self.gate.state);
            state.batch.push(invoice.to_owned());
            if state.batch.len() >= gate.threshold {
                Some(self.gate.take(&mut state))
            } else {
                if state.batch.len() == 1 && !gate.interval.is_zero() {
                    state.first_at = Some(Instant::now());
                    self.gate.changed.notify_all();
                }
                None
            }
        };
        if let Some(batch) = ready {
            self.fire_batch(batch);
        }
    }

    /// Files the pending batch when its oldest invoice has waited the interval.
    fn timer(&self) {
        let Some(gate) = &self.cfg.gate else { return };
        let mut state = lock(&self.gate.state);
        loop {
            if state.closed {
                return;
            }
            let due = state.first_at.map(|t| t + gate.interval);
            match due {
                Some(due) if !self.cancelled() => {
                    let now = Instant::now();
                    if now >= due {
                        let batch = self.gate.take(&mut state);
                        drop(state);
                        self.fire_batch(batch);
                        state = lock(&self.gate.state);
                    } else {
                        state = self
                            .gate
                            .changed
                            .wait_timeout(state, due - now)
                            .unwrap_or_else(|e| e.into_inner())
                            .0;
                    }
                }
                _ => {
                    state = self
                        .gate
                        .changed
                        .wait(state)
                        .unwrap_or_else(|e| e.into_inner());
                }
            }
        }
    }

    /// Files one batch. One at a time: the caller of the filing holds a
    /// per-cargue lease, so two uploads of one run must never overlap.
    fn fire_batch(&self, mut batch: Vec<String>) {
        let (Some(fire), Some(gate)) = (self.hooks.fire, &self.cfg.gate) else {
            return;
        };
        if batch.is_empty() {
            return;
        }
        batch.sort();
        let _serial = lock(&self.fire_lock);
        let started = Instant::now();
        let fired = fire(&batch);
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut event = Event::new("radicacion").step(gate.label).millis(ms);
        event.batch = batch.len();
        event.invoices = batch.clone();
        let mut abort = false;
        {
            let mut state = lock(&self.state);
            match &fired {
                Fired::Done(summary) => {
                    state.counts.done += batch.len();
                    event.status = Status::Ok.as_str().to_owned();
                    event.reason = summary.clone();
                }
                Fired::OtherMachine(reason) => {
                    state.counts.skipped += batch.len();
                    event.status = Status::Skipped.as_str().to_owned();
                    event.reason = reason.clone();
                }
                Fired::Failed(error) => {
                    state
                        .error
                        .get_or_insert_with(|| format!("radicacion: {error}"));
                    state.counts.failed += 1;
                    state.worst = state.worst.max(Status::Failed);
                    event.status = Status::Failed.as_str().to_owned();
                    event.error = error.clone();
                    abort = !self.cfg.continue_on_error;
                }
            }
        }
        if abort {
            self.cancel();
        }
        self.events.emit(event);
    }
}

fn count(state: &mut State, status: Status) {
    match status {
        Status::Manual => state.counts.manual += 1,
        Status::Excluded => state.counts.excluded += 1,
        Status::Failed => state.counts.failed += 1,
        Status::Ok | Status::Skipped => {}
    }
    if matches!(status, Status::Manual | Status::Excluded | Status::Failed) {
        state.worst = state.worst.max(status);
    }
}
