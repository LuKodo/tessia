//! Several machines running `proceso run` on the same cargue split the work:
//! the phases claim every (invoice, phase) in `proceso_estado`, and the
//! orchestrator treats an invoice another machine owns as skipped, never as
//! a failure. Both "machines" share one database and bucket here.

#[path = "proceso_support.rs"]
mod support;

use std::collections::HashSet;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use support::*;
use tessia::context::AppContext;
use tessia::domain::Phase;
use tessia::phases::convenio_preparar::{PrepareRequest, commit_preparation, plan_preparation};
use tessia::phases::proceso::{Mode, execute};

const PHASES: [(Phase, &str); 6] = [
    (Phase::TheFactory, "thefactory"),
    (Phase::FevValor, "fev"),
    (Phase::Derechos, "derechos"),
    (Phase::Rips, "rips"),
    (Phase::Cuv, "cuv"),
    (Phase::Soporte, "soporte"),
];

fn prepared(env: &Env, invoices: usize) {
    let path = write_csv(env.dir.path(), &format!("{ID}.csv"), &pos_rows(invoices));
    let request = PrepareRequest {
        source: path.to_string_lossy().into_owned(),
        max_amount: Some("250000".into()),
        ..PrepareRequest::default()
    };
    let plan = plan_preparation(&env.ctx, &request, None).expect("plan");
    commit_preparation(&env.ctx, &plan).expect("commit");
}

fn slow_runner(
    log: &Log,
    rad: &Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    pause: Duration,
) -> tessia::phases::proceso::PhaseRunner {
    let mut runner = tessia::phases::proceso::PhaseRunner::new();
    for (phase, name) in PHASES {
        runner = runner.with(
            name,
            claiming_phase(phase, name, log.clone(), always_done(), pause),
        );
    }
    runner.with("radicacion", recording_radicacion(rad.clone()))
}

#[test]
fn two_orchestrators_over_the_same_cargue_never_process_an_invoice_twice() {
    let env = setup();
    prepared(&env, 30);
    let log = Log::default();
    let (rad_a, rad_b) = (batches(), batches());
    let barrier = Arc::new(Barrier::new(2));
    let mut cfg = config(ID);
    cfg.batch_radicacion = 7;

    let results: Vec<(String, usize, usize)> = std::thread::scope(|scope| {
        let handles: Vec<_> = [("machine-a", rad_a.clone()), ("machine-b", rad_b.clone())]
            .into_iter()
            .map(|(owner, rad)| {
                let ctx: AppContext = env.ctx.clone().with_owner(owner);
                let (cfg, log, barrier) = (cfg.clone(), log.clone(), barrier.clone());
                scope.spawn(move || {
                    let out = capture(Mode::Json);
                    let runner = slow_runner(&log, &rad, Duration::from_millis(2));
                    barrier.wait();
                    let summary = execute(&ctx, &cfg, &runner, &out.events).expect("run");
                    assert_eq!(
                        summary.finished.failed, 0,
                        "{owner}: a skip is not a failure"
                    );
                    assert_eq!(summary.finished.replaced, 0);
                    (
                        owner.to_owned(),
                        summary.finished.done,
                        summary.finished.skipped,
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("join"))
            .collect()
    });

    // Every (phase, invoice) was worked exactly once across both machines.
    for (_, phase) in PHASES {
        for n in 1..=30 {
            assert_eq!(log.count(phase, &format!("EVS{n}")), 1, "{phase} EVS{n}");
        }
    }
    assert_eq!(log.all().len(), 6 * 30);
    // Both machines did real work (the cargue was split) and nothing was lost.
    let owners: HashSet<String> = log.all().into_iter().map(|w| w.owner).collect();
    assert_eq!(owners.len(), 2, "{results:?}");
    let mut filed = flatten(&rad_a);
    filed.extend(flatten(&rad_b));
    let filed: HashSet<String> = filed.into_iter().collect();
    assert_eq!(filed.len(), 30, "every invoice reached radicación");
    let skipped: usize = results.iter().map(|r| r.2).sum();
    assert!(
        skipped > 0,
        "the machines met on some invoices: {results:?}"
    );
}

#[test]
fn a_machine_joining_a_running_cargue_takes_the_invoices_nobody_has_claimed() {
    let env = setup();
    prepared(&env, 24);
    let log = Log::default();
    let (rad_a, rad_b) = (batches(), batches());
    let cfg = config(ID);
    std::thread::scope(|scope| {
        let first = {
            let ctx = env.ctx.clone().with_owner("cron");
            let (cfg, log, rad) = (cfg.clone(), log.clone(), rad_a.clone());
            scope.spawn(move || {
                let out = capture(Mode::Json);
                execute(
                    &ctx,
                    &cfg,
                    &slow_runner(&log, &rad, Duration::from_millis(40)),
                    &out.events,
                )
                .expect("cron run");
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        let ctx = env.ctx.clone().with_owner("manual");
        let out = capture(Mode::Json);
        let summary = execute(
            &ctx,
            &config(ID),
            &slow_runner(&log, &rad_b, Duration::from_millis(40)),
            &out.events,
        )
        .expect("manual run");
        assert_eq!(summary.finished.failed, 0);
        first.join().expect("cron joined");
    });
    for (_, phase) in PHASES {
        for n in 1..=24 {
            assert_eq!(log.count(phase, &format!("EVS{n}")), 1, "{phase} EVS{n}");
        }
    }
    let by_manual = log.all().iter().filter(|w| w.owner == "manual").count();
    assert!(by_manual > 0, "the second machine contributed");
    // No cargue-level lease exists: nothing named after the cargue is held.
    assert_eq!(
        env.ctx
            .leases()
            .holder(&format!("proceso:{ID}"))
            .expect("holder"),
        None
    );
}

#[test]
fn an_invoice_the_other_machine_owns_is_not_replaced() {
    let env = setup();
    let path = write_csv(env.dir.path(), &format!("{ID}.csv"), &pos_rows(6));
    let mut cfg = config(&path.to_string_lossy());
    cfg.limit = 3;
    // Another machine already owns EVS1 in thefactory (EN_PROGRESO).
    let other = env.ctx.clone().with_owner("other");
    let claimed = other
        .proceso()
        .claim_available(
            Phase::TheFactory,
            &[tessia::stores::proceso::ClaimItem::new("EVS1")],
            &other.claim_options(ID),
        )
        .expect("claim");
    assert_eq!(claimed.len(), 1);
    let (log, rad) = (Log::default(), batches());
    let out = capture(Mode::Json);
    let summary = execute(&env.ctx, &cfg, &fake_runner(&log, &rad), &out.events).expect("run");
    assert_eq!(summary.finished.skipped, 1);
    assert_eq!(summary.finished.replaced, 0, "a skip is not a drop");
    assert_eq!(summary.finished.done, 2);
    assert!(log.path("EVS1").is_empty());
    assert!(out.of("invoice_replace").is_empty());
    let skipped = out
        .of("invoice_step")
        .into_iter()
        .find(|e| e.status == "SKIPPED")
        .expect("skipped step");
    assert_eq!(
        (skipped.invoice.as_str(), skipped.reason.as_str()),
        ("EVS1", OTHER_MACHINE)
    );
}
