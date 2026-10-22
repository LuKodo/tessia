//! Engine tests with fake phases (no database).

use std::collections::HashSet;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("sink").extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Collects every event in emission order.
fn recorder() -> (Events, Arc<Mutex<Vec<Event>>>) {
    let seen: Arc<Mutex<Vec<Event>>> = Arc::default();
    let tap = seen.clone();
    let events = Events::new(
        super::super::events::Mode::Json,
        Box::new(Sink::default()),
        false,
    )
    .with_tap(move |e| tap.lock().expect("events").push(e.clone()));
    (events, seen)
}

fn spec(name: &'static str, workers: usize, next: Option<&'static str>) -> PhaseSpec {
    PhaseSpec {
        name,
        label: name,
        workers,
        next,
    }
}

fn config(phases: Vec<PhaseSpec>, invoices: &[&str]) -> PipelineConfig {
    PipelineConfig {
        entry: phases.first().map(|p| p.name),
        phases,
        invoices: invoices.iter().map(|s| (*s).to_owned()).collect(),
        continue_on_error: false,
        gate: None,
        target: 0,
        replace_cap: 0,
        breaker: 0,
    }
}

fn ok(_: &str, _: &str) -> Verdict {
    Verdict::ok()
}

fn of_type(events: &Arc<Mutex<Vec<Event>>>, kind: &str) -> Vec<Event> {
    events
        .lock()
        .expect("events")
        .iter()
        .filter(|e| e.kind == kind)
        .cloned()
        .collect()
}

fn run_end(events: &Arc<Mutex<Vec<Event>>>) -> Event {
    let ends = of_type(events, "run_end");
    assert_eq!(ends.len(), 1, "exactly one run_end");
    ends[0].clone()
}

fn go(
    cfg: &PipelineConfig,
    run: RunFn<'_>,
    fire: Option<FireFn<'_>>,
    supply: Option<SupplyFn<'_>>,
) -> (Finished, Arc<Mutex<Vec<Event>>>) {
    let (events, seen) = recorder();
    let hooks = Hooks { run, fire, supply };
    (run_pipeline(cfg, &hooks, &events), seen)
}

fn manual(reason: &str) -> Verdict {
    Verdict::new(Status::Manual, reason)
}

#[test]
fn every_invoice_visits_the_phases_in_order_and_run_end_tallies_them() {
    let cfg = config(
        vec![spec("a", 2, Some("b")), spec("b", 2, None)],
        &["i1", "i2", "i3"],
    );
    let (finished, seen) = go(&cfg, &ok, None, None);
    assert_eq!(finished.worst, Status::Ok);
    assert_eq!(finished.done, 3);
    let steps = of_type(&seen, "invoice_step");
    for invoice in ["i1", "i2", "i3"] {
        let phases: Vec<&str> = steps
            .iter()
            .filter(|e| e.invoice == invoice)
            .map(|e| e.phase.as_str())
            .collect();
        assert_eq!(phases, ["a", "b"], "{invoice}");
    }
    assert_eq!(of_type(&seen, "run_start")[0].total, 3);
    let end = run_end(&seen);
    assert_eq!((end.done, end.status.as_str()), (3, "OK"));
    assert!(end.phase_ms.contains_key("a") && end.phase_ms.contains_key("b"));
}

#[test]
fn a_slow_invoice_never_holds_back_the_others() {
    let log: Mutex<Vec<String>> = Mutex::default();
    let run = |phase: &str, invoice: &str| {
        if phase == "a" && invoice == "slow" {
            std::thread::sleep(Duration::from_millis(300));
        }
        log.lock().expect("log").push(format!("{phase}:{invoice}"));
        Verdict::ok()
    };
    let cfg = config(
        vec![spec("a", 4, Some("b")), spec("b", 4, None)],
        &["slow", "f1", "f2"],
    );
    go(&cfg, &run, None, None);
    let log = log.lock().expect("log");
    let at = |entry: &str| log.iter().position(|e| e == entry).expect(entry);
    assert!(
        at("b:f1") < at("a:slow") && at("b:f2") < at("a:slow"),
        "{log:?}"
    );
}

#[test]
fn a_phase_never_runs_more_invoices_than_its_workers() {
    let (active, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let run = |_: &str, _: &str| {
        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
        peak.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(15));
        active.fetch_sub(1, Ordering::SeqCst);
        Verdict::ok()
    };
    let names: Vec<String> = (0..20).map(|n| format!("i{n:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let cfg = config(vec![spec("a", 3, None)], &refs);
    go(&cfg, &run, None, None);
    assert!(peak.load(Ordering::SeqCst) <= 3);
    assert!(
        peak.load(Ordering::SeqCst) >= 2,
        "the pool is actually used"
    );
}

#[test]
fn drop_outs_are_counted_and_the_worst_status_wins() {
    let run = |_: &str, invoice: &str| match invoice {
        "m" => manual("revisar"),
        "e" => Verdict::new(Status::Excluded, "regla"),
        "f" => Verdict::new(Status::Failed, "FAILED"),
        _ => Verdict::ok(),
    };
    let cfg = config(vec![spec("a", 2, None)], &["m", "e", "f", "g"]);
    let (finished, seen) = go(&cfg, &run, None, None);
    assert_eq!(finished.worst, Status::Failed);
    assert!(
        finished.error.is_none(),
        "a recorded failure is not systemic"
    );
    let end = run_end(&seen);
    assert_eq!(
        (end.done, end.manual, end.excluded, end.failed),
        (1, 1, 1, 1)
    );
}

#[test]
fn fev_routes_value_differences_to_conciliar_and_back_to_the_successor() {
    let run = |phase: &str, invoice: &str| match (phase, invoice) {
        ("fev", "diff") => Verdict {
            route: Some("conciliar"),
            ..Verdict::new(Status::Ok, "VALOR_DIFERENTE")
        },
        _ => Verdict::ok(),
    };
    let cfg = config(
        vec![
            spec("fev", 1, Some("derechos")),
            spec("derechos", 1, None),
            spec("conciliar", 1, Some("derechos")),
        ],
        &["diff", "same"],
    );
    let (_, seen) = go(&cfg, &run, None, None);
    let path = |invoice: &str| -> Vec<String> {
        of_type(&seen, "invoice_step")
            .into_iter()
            .filter(|e| e.invoice == invoice)
            .map(|e| e.phase)
            .collect()
    };
    assert_eq!(path("diff"), ["fev", "conciliar", "derechos"]);
    assert_eq!(path("same"), ["fev", "derechos"]);
}

#[test]
fn a_route_to_an_inactive_phase_drops_the_invoice_for_manual_review() {
    let run = |_: &str, _: &str| Verdict {
        route: Some("conciliar"),
        ..Verdict::new(Status::Ok, "VALOR_DIFERENTE")
    };
    let cfg = config(
        vec![spec("fev", 1, Some("derechos")), spec("derechos", 1, None)],
        &["x"],
    );
    let (finished, _) = go(&cfg, &run, None, None);
    assert_eq!((finished.manual, finished.done), (1, 0));
    assert_eq!(finished.worst, Status::Manual);
}

fn systemic(phase: &str, invoice: &str) -> Verdict {
    if phase == "a" && invoice == "bad" {
        Verdict {
            error: Some("db down".into()),
            ..Verdict::new(Status::Failed, "")
        }
    } else {
        Verdict::ok()
    }
}

#[test]
fn a_systemic_error_aborts_the_run_and_names_phase_and_invoice() {
    // One worker keeps the order deterministic: "bad" runs first.
    let cfg = config(
        vec![spec("a", 1, Some("b")), spec("b", 1, None)],
        &["bad", "z1", "z2", "z3"],
    );
    let (finished, seen) = go(&cfg, &systemic, None, None);
    assert_eq!(
        finished.error.as_deref(),
        Some("phase \"a\", invoice bad: db down")
    );
    assert_eq!(finished.worst, Status::Failed);
    assert!(
        of_type(&seen, "invoice_step").len() < 7,
        "the rest of the invoices were not processed"
    );
    assert_eq!(finished.done, 0);
}

#[test]
fn continue_on_error_keeps_going_but_still_reports_the_error() {
    let mut cfg = config(
        vec![spec("a", 1, Some("b")), spec("b", 1, None)],
        &["bad", "z1", "z2"],
    );
    cfg.continue_on_error = true;
    let (finished, _) = go(&cfg, &systemic, None, None);
    assert_eq!(finished.done, 2);
    assert!(finished.error.is_some());
}

#[test]
fn a_fatal_error_aborts_even_with_continue_on_error() {
    let run = |_: &str, invoice: &str| {
        if invoice == "bad" {
            Verdict {
                error: Some("credenciales".into()),
                fatal: true,
                ..Verdict::new(Status::Failed, "")
            }
        } else {
            std::thread::sleep(Duration::from_millis(5));
            Verdict::ok()
        }
    };
    let mut cfg = config(vec![spec("a", 1, None)], &["bad", "z1", "z2", "z3"]);
    cfg.continue_on_error = true;
    let (finished, _) = go(&cfg, &run, None, None);
    assert_eq!(finished.done, 0, "nothing runs after a fatal error");
    assert!(finished.error.is_some());
}

#[test]
fn a_resumable_error_does_not_abort_but_is_reported() {
    let run = |_: &str, invoice: &str| {
        if invoice == "bad" {
            Verdict {
                error: Some("medicar caído".into()),
                resumable: true,
                ..Verdict::new(Status::Failed, "")
            }
        } else {
            Verdict::ok()
        }
    };
    let cfg = config(vec![spec("a", 1, None)], &["bad", "z1", "z2"]);
    let (finished, _) = go(&cfg, &run, None, None);
    assert_eq!(finished.done, 2);
    assert!(
        finished
            .error
            .as_deref()
            .is_some_and(|e| e.contains("medicar caído"))
    );
}

#[test]
fn a_panicking_phase_fails_its_invoice_and_aborts_instead_of_hanging() {
    let run = |_: &str, invoice: &str| -> Verdict {
        if invoice == "boom" {
            panic!("fake phase panic");
        }
        Verdict::ok()
    };
    let cfg = config(vec![spec("a", 1, None)], &["boom"]);
    let (finished, _) = go(&cfg, &run, None, None);
    assert_eq!(finished.failed, 1);
    assert!(finished.error.is_some());
}

#[test]
fn an_invoice_owned_by_another_machine_is_skipped_not_failed_or_replaced() {
    let run = |phase: &str, invoice: &str| {
        if phase == "a" && invoice == "theirs" {
            Verdict::new(Status::Skipped, "EN_PROCESO_OTRA_MAQUINA")
        } else {
            Verdict::ok()
        }
    };
    let supplied = AtomicUsize::new(0);
    let supply = |_: usize| {
        supplied.fetch_add(1, Ordering::SeqCst);
        Ok(vec!["r1".to_owned()])
    };
    let mut cfg = config(
        vec![spec("a", 1, Some("b")), spec("b", 1, None)],
        &["theirs", "mine"],
    );
    cfg.target = 2;
    cfg.replace_cap = 5;
    let (finished, seen) = go(&cfg, &run, None, Some(&supply));
    assert_eq!(
        (finished.skipped, finished.done, finished.failed),
        (1, 1, 0)
    );
    assert_eq!(finished.worst, Status::Ok);
    assert_eq!(supplied.load(Ordering::SeqCst), 0);
    assert_eq!(run_end(&seen).skipped, 1);
    let step = of_type(&seen, "invoice_step")
        .into_iter()
        .find(|e| e.invoice == "theirs")
        .expect("step");
    assert_eq!(
        (step.status.as_str(), step.reason.as_str()),
        ("SKIPPED", "EN_PROCESO_OTRA_MAQUINA")
    );
}

fn gate(threshold: usize, interval: Duration) -> Option<GateConfig> {
    Some(GateConfig {
        threshold,
        interval,
        label: "radicacion run",
    })
}

#[derive(Default)]
struct Fires {
    batches: Mutex<Vec<Vec<String>>>,
    active: AtomicUsize,
    peak: AtomicUsize,
}

impl Fires {
    fn fire(&self, batch: &[String]) -> Fired {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(10));
        self.batches.lock().expect("batches").push(batch.to_vec());
        self.active.fetch_sub(1, Ordering::SeqCst);
        Fired::Done(String::new())
    }

    fn batches(&self) -> Vec<Vec<String>> {
        self.batches.lock().expect("batches").clone()
    }
}

#[test]
fn the_gate_files_full_batches_sorted_and_flushes_the_rest_at_the_end() {
    let names: Vec<String> = (1..=7).rev().map(|n| format!("i{n}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut cfg = config(vec![spec("a", 1, None)], &refs);
    cfg.gate = gate(3, Duration::ZERO);
    let fires = Fires::default();
    let fire = |b: &[String]| fires.fire(b);
    let (finished, seen) = go(&cfg, &ok, Some(&fire), None);
    let sizes: Vec<usize> = fires.batches().iter().map(Vec::len).collect();
    assert_eq!(sizes, [3, 3, 1]);
    for batch in fires.batches() {
        let mut sorted = batch.clone();
        sorted.sort();
        assert_eq!(batch, sorted);
    }
    assert_eq!(finished.done, 7, "done counts the invoices filed");
    let events = of_type(&seen, "radicacion");
    assert_eq!(events.len(), 3);
    assert!(
        events
            .iter()
            .all(|e| e.status == "OK" && e.step == "radicacion run")
    );
    assert_eq!(events[0].invoices.len(), events[0].batch);
}

#[test]
fn a_partial_batch_is_filed_when_its_oldest_invoice_has_waited_the_interval() {
    let order: Mutex<Vec<String>> = Mutex::default();
    let run = |_: &str, invoice: &str| {
        if invoice == "slow" {
            std::thread::sleep(Duration::from_millis(700));
            order.lock().expect("order").push("slow-done".into());
        }
        Verdict::ok()
    };
    let fire = |batch: &[String]| {
        order
            .lock()
            .expect("order")
            .push(format!("fire:{}", batch.join(",")));
        Fired::Done(String::new())
    };
    let mut cfg = config(vec![spec("a", 2, None)], &["f1", "f2", "slow"]);
    cfg.gate = gate(100, Duration::from_millis(100));
    go(&cfg, &run, Some(&fire), None);
    let order = order.lock().expect("order").clone();
    assert_eq!(order, ["fire:f1,f2", "slow-done", "fire:slow"], "{order:?}");
}

#[test]
fn interval_zero_disables_the_timer() {
    let order: Mutex<Vec<String>> = Mutex::default();
    let run = |_: &str, invoice: &str| {
        if invoice == "slow" {
            std::thread::sleep(Duration::from_millis(300));
            order.lock().expect("order").push("slow-done".into());
        }
        Verdict::ok()
    };
    let fire = |batch: &[String]| {
        order
            .lock()
            .expect("order")
            .push(format!("fire:{}", batch.len()));
        Fired::Done(String::new())
    };
    let mut cfg = config(vec![spec("a", 2, None)], &["f1", "f2", "slow"]);
    cfg.gate = gate(100, Duration::ZERO);
    go(&cfg, &run, Some(&fire), None);
    assert_eq!(*order.lock().expect("order"), ["slow-done", "fire:3"]);
}

#[test]
fn uploads_never_overlap_even_with_the_timer_and_the_threshold() {
    let names: Vec<String> = (0..30).map(|n| format!("i{n:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let run = |_: &str, _: &str| {
        std::thread::sleep(Duration::from_millis(3));
        Verdict::ok()
    };
    let mut cfg = config(vec![spec("a", 8, None)], &refs);
    cfg.gate = gate(2, Duration::from_millis(5));
    let fires = Fires::default();
    let fire = |b: &[String]| fires.fire(b);
    let (finished, _) = go(&cfg, &run, Some(&fire), None);
    assert_eq!(fires.peak.load(Ordering::SeqCst), 1);
    assert_eq!(finished.done, 30);
    let filed: usize = fires.batches().iter().map(Vec::len).sum();
    assert_eq!(filed, 30);
}

#[test]
fn a_failed_batch_is_a_failed_run_and_a_batch_left_to_another_machine_is_not() {
    let mut cfg = config(vec![spec("a", 1, None)], &["i1", "i2"]);
    cfg.gate = gate(10, Duration::ZERO);
    let fail = |_: &[String]| Fired::Failed("portal caído".into());
    let (finished, seen) = go(&cfg, &ok, Some(&fail), None);
    assert_eq!(finished.worst, Status::Failed);
    assert_eq!(finished.error.as_deref(), Some("radicacion: portal caído"));
    let event = &of_type(&seen, "radicacion")[0];
    assert_eq!(
        (event.status.as_str(), event.error.as_str()),
        ("FAILED", "portal caído")
    );

    let other = |_: &[String]| Fired::OtherMachine("otra máquina".into());
    let (finished, seen) = go(&cfg, &ok, Some(&other), None);
    assert_eq!((finished.error, finished.worst), (None, Status::Ok));
    assert_eq!(finished.skipped, 2);
    assert_eq!(of_type(&seen, "radicacion")[0].status, "SKIPPED");
}

#[test]
fn without_the_gate_cleared_invoices_just_finish() {
    let cfg = config(vec![spec("a", 1, None)], &["i1", "i2"]);
    let (finished, seen) = go(&cfg, &ok, None, None);
    assert_eq!(finished.done, 2);
    assert!(of_type(&seen, "radicacion").is_empty());
}

#[test]
fn with_no_per_invoice_phase_everything_goes_straight_to_the_gate() {
    let mut cfg = config(Vec::new(), &["i1", "i2", "i3"]);
    cfg.gate = gate(2, Duration::ZERO);
    let fires = Fires::default();
    let fire = |b: &[String]| fires.fire(b);
    let (finished, _) = go(&cfg, &ok, Some(&fire), None);
    assert_eq!(finished.done, 3);
    let sizes: Vec<usize> = fires.batches().iter().map(Vec::len).collect();
    assert_eq!(sizes, [2, 1]);
}

/// A deterministic supplier over a pool; it records its calls.
struct Pool {
    pool: Mutex<Vec<String>>,
    calls: Mutex<Vec<usize>>,
    error: Option<&'static str>,
}

impl Pool {
    fn new(names: &[&str]) -> Self {
        Self {
            pool: Mutex::new(names.iter().map(|s| (*s).to_owned()).collect()),
            calls: Mutex::default(),
            error: None,
        }
    }

    fn supply(&self, n: usize) -> Result<Vec<String>> {
        self.calls.lock().expect("calls").push(n);
        if let Some(error) = self.error {
            return Err(anyhow::anyhow!(error));
        }
        let mut pool = self.pool.lock().expect("pool");
        let take = n.min(pool.len());
        Ok(pool.drain(..take).collect())
    }

    fn calls(&self) -> Vec<usize> {
        self.calls.lock().expect("calls").clone()
    }
}

fn replacing(mut cfg: PipelineConfig, target: usize, cap: usize) -> PipelineConfig {
    cfg.target = target;
    cfg.replace_cap = cap;
    cfg
}

fn cleared_in(seen: &Arc<Mutex<Vec<Event>>>, phase: &str) -> usize {
    of_type(seen, "invoice_step")
        .iter()
        .filter(|e| e.phase == phase && e.status == "OK")
        .map(|e| e.invoice.clone())
        .collect::<HashSet<_>>()
        .len()
}

#[test]
fn manual_drops_are_replaced_until_the_target_clears() {
    let run = |phase: &str, invoice: &str| {
        if phase == "b" && (invoice == "i2" || invoice == "i4") {
            manual("needs a person")
        } else {
            Verdict::ok()
        }
    };
    let pool = Pool::new(&["r1", "r2", "r3", "r4", "r5"]);
    let supply = |n: usize| pool.supply(n);
    let cfg = replacing(
        config(
            vec![spec("a", 2, Some("b")), spec("b", 2, None)],
            &["i1", "i2", "i3", "i4", "i5"],
        ),
        5,
        5,
    );
    let (finished, seen) = go(&cfg, &run, None, Some(&supply));
    assert_eq!(cleared_in(&seen, "b"), 5);
    assert_eq!((finished.replaced, finished.entered), (2, 7));
    assert_eq!(finished.worst, Status::Manual);
    let end = run_end(&seen);
    assert_eq!(
        (end.replaced, end.total, end.done, end.manual),
        (2, 7, 5, 2)
    );
    assert_eq!(
        of_type(&seen, "run_start")[0].total,
        5,
        "the initial window"
    );
    let mut covered: Vec<String> = of_type(&seen, "invoice_replace")
        .into_iter()
        .map(|e| {
            assert_eq!(e.phase, "b");
            e.reason
        })
        .collect();
    covered.sort();
    assert_eq!(covered, ["replaces i2", "replaces i4"]);
}

#[test]
fn every_kind_of_drop_triggers_a_replacement() {
    let kinds = [
        Verdict::new(Status::Failed, "download failed"),
        Verdict::new(Status::Excluded, "rule"),
        Verdict {
            error: Some("db".into()),
            ..Verdict::new(Status::Failed, "")
        },
    ];
    for bad in kinds {
        let run = |_: &str, invoice: &str| {
            if invoice == "bad" {
                bad.clone()
            } else {
                Verdict::ok()
            }
        };
        let pool = Pool::new(&["r1", "r2"]);
        let supply = |n: usize| pool.supply(n);
        let mut cfg = replacing(config(vec![spec("a", 1, None)], &["bad", "good"]), 2, 2);
        cfg.continue_on_error = true;
        let (finished, seen) = go(&cfg, &run, None, Some(&supply));
        assert_eq!(cleared_in(&seen, "a"), 2, "{bad:?}");
        assert_eq!((finished.replaced, finished.entered), (1, 3));
        assert_eq!(of_type(&seen, "invoice_replace")[0].reason, "replaces bad");
    }
}

#[test]
fn a_replacement_that_drops_is_replaced_in_turn() {
    let run = |_: &str, invoice: &str| {
        if invoice == "i1" || invoice == "r1" {
            manual("x")
        } else {
            Verdict::ok()
        }
    };
    let pool = Pool::new(&["r1", "r2", "r3"]);
    let supply = |n: usize| pool.supply(n);
    let cfg = replacing(config(vec![spec("a", 1, None)], &["i1"]), 1, 3);
    let (finished, seen) = go(&cfg, &run, None, Some(&supply));
    assert_eq!(
        (finished.done, finished.manual, finished.replaced),
        (1, 2, 2)
    );
    let chain: Vec<String> = of_type(&seen, "invoice_replace")
        .into_iter()
        .map(|e| format!("{}:{}", e.invoice, e.reason))
        .collect();
    assert_eq!(chain, ["r1:replaces i1", "r2:replaces r1"]);
}

#[test]
fn an_exhausted_pool_stops_calling_the_supplier() {
    let run = |_: &str, invoice: &str| {
        if ["i1", "i2", "i3"].contains(&invoice) {
            manual("x")
        } else {
            Verdict::ok()
        }
    };
    let pool = Pool::new(&["r1"]);
    let supply = |n: usize| pool.supply(n);
    let cfg = replacing(
        config(vec![spec("a", 1, None)], &["i1", "i2", "i3", "i4"]),
        4,
        1,
    );
    let (finished, _) = go(&cfg, &run, None, Some(&supply));
    assert_eq!(finished.done, 2, "i4 and r1");
    assert_eq!(pool.calls().len(), 1);
    assert_eq!(finished.replaced, 1);
}

#[test]
fn a_supplier_error_stops_replacement_quietly_and_reports_why() {
    let run = |_: &str, invoice: &str| {
        if invoice == "i1" {
            manual("x")
        } else {
            Verdict::ok()
        }
    };
    let mut pool = Pool::new(&["r1"]);
    pool.error = Some("mipres abortó");
    let supply = |n: usize| pool.supply(n);
    let cfg = replacing(config(vec![spec("a", 1, None)], &["i1", "i2"]), 2, 5);
    let (finished, seen) = go(&cfg, &run, None, Some(&supply));
    assert!(finished.error.is_none(), "the run itself is not aborted");
    assert_eq!(finished.done, 1);
    let stopped = of_type(&seen, "replace_stopped");
    assert_eq!(stopped.len(), 1);
    assert!(stopped[0].reason.starts_with("supply error: mipres abortó"));
}

#[test]
fn the_circuit_breaker_pauses_replacement_after_consecutive_drops() {
    let run = |phase: &str, _: &str| {
        if phase == "a" {
            manual("x")
        } else {
            Verdict::ok()
        }
    };
    let names: Vec<String> = (0..30).map(|n| format!("r{n:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let pool = Pool::new(&refs);
    let supply = |n: usize| pool.supply(n);
    let mut cfg = replacing(config(vec![spec("a", 1, None)], &["i1", "i2"]), 2, 30);
    cfg.breaker = 3;
    let (finished, seen) = go(&cfg, &run, None, Some(&supply));
    let stopped = of_type(&seen, "replace_stopped");
    assert_eq!(stopped.len(), 1);
    assert!(
        stopped[0]
            .reason
            .starts_with("3 consecutive invoices stopped before radicación")
    );
    assert_eq!(finished.manual, finished.entered, "everything dropped");
    assert!(finished.entered < 32, "the whole pool was not walked");
}

#[test]
fn a_clear_resets_the_breaker_streak() {
    // Drops and clears alternate: the streak never reaches 2.
    let run = |_: &str, invoice: &str| {
        if invoice.starts_with('d') {
            manual("x")
        } else {
            Verdict::ok()
        }
    };
    let pool = Pool::new(&["d1", "g1", "d2", "g2", "d3", "g3"]);
    let supply = |n: usize| pool.supply(n);
    let mut cfg = replacing(config(vec![spec("a", 1, None)], &["d0", "g0"]), 5, 6);
    cfg.breaker = 2;
    let (_, seen) = go(&cfg, &run, None, Some(&supply));
    assert!(of_type(&seen, "replace_stopped").is_empty());
}

#[test]
fn radicacion_outcomes_are_never_replaced() {
    let mut cfg = replacing(config(vec![spec("a", 1, None)], &["i1", "i2"]), 2, 5);
    cfg.gate = gate(10, Duration::ZERO);
    let pool = Pool::new(&["r1"]);
    let supply = |n: usize| pool.supply(n);
    let fail = |_: &[String]| Fired::Failed("x".into());
    let (finished, _) = go(&cfg, &ok, Some(&fail), Some(&supply));
    assert!(pool.calls().is_empty());
    assert_eq!(finished.replaced, 0);
}

#[test]
fn a_systemic_error_cancels_without_replacing() {
    let pool = Pool::new(&["r1", "r2"]);
    let supply = |n: usize| pool.supply(n);
    let cfg = replacing(config(vec![spec("a", 1, None)], &["bad", "z"]), 2, 2);
    let (finished, _) = go(&cfg, &systemic, None, Some(&supply));
    assert!(finished.error.is_some());
    assert!(pool.calls().is_empty());
}

#[test]
fn concurrent_drops_serialize_the_supplier_and_respect_the_cap() {
    let running = AtomicUsize::new(0);
    let overlaps = AtomicUsize::new(0);
    let pool = Pool::new(&["r1", "r2", "r3"]);
    let supply = |n: usize| {
        if running.fetch_add(1, Ordering::SeqCst) > 0 {
            overlaps.fetch_add(1, Ordering::SeqCst);
        }
        std::thread::sleep(Duration::from_millis(10));
        let got = pool.supply(n);
        running.fetch_sub(1, Ordering::SeqCst);
        got
    };
    let run = |_: &str, invoice: &str| {
        if invoice.starts_with('i') {
            manual("x")
        } else {
            Verdict::ok()
        }
    };
    let cfg = replacing(
        config(
            vec![spec("a", 8, None)],
            &["i1", "i2", "i3", "i4", "i5", "i6"],
        ),
        6,
        3,
    );
    let (finished, _) = go(&cfg, &run, None, Some(&supply));
    assert_eq!(
        overlaps.load(Ordering::SeqCst),
        0,
        "supply calls overlapped"
    );
    assert_eq!(finished.replaced, 3, "never more than the cap");
    assert_eq!(finished.done, 3);
}

#[test]
fn replacement_off_without_a_target_or_a_supplier() {
    let run = |_: &str, invoice: &str| {
        if invoice == "i1" {
            manual("x")
        } else {
            Verdict::ok()
        }
    };
    let pool = Pool::new(&["r1"]);
    let supply = |n: usize| pool.supply(n);
    let (finished, seen) = go(
        &config(vec![spec("a", 1, None)], &["i1"]),
        &run,
        None,
        Some(&supply),
    );
    assert!(pool.calls().is_empty());
    let end = run_end(&seen);
    assert_eq!((end.total, end.replaced, finished.manual), (0, 0, 1));
}
