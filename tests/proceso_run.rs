//! `proceso run` end to end: real seeding (`convenio preparar`), real stores
//! and a throwaway Postgres; the phases are fakes that claim in
//! `proceso_estado` like the real ones. No external system is contacted.

#[path = "proceso_support.rs"]
mod support;

use std::sync::Arc;

use anyhow::anyhow;
use support::*;
use tessia::domain::radicacion::{RadState, RadStatus, Step};
use tessia::domain::{Outcome, Phase, PhaseState};
use tessia::phases::conciliar::plan::{Detail, Incomplete};
use tessia::phases::cuv::MissingRipsJsonError;
use tessia::phases::derechos::DerechosError;
use tessia::phases::proceso::{Mode, execute, steps};

const MAINLINE: [&str; 6] = ["thefactory", "fev", "derechos", "rips", "cuv", "soporte"];

fn run_with(
    env: &Env,
    cfg: &tessia::phases::proceso::RunConfig,
    runner: &tessia::phases::proceso::PhaseRunner,
) -> (anyhow::Result<tessia::phases::proceso::RunSummary>, Capture) {
    let out = capture(Mode::Json);
    (execute(&env.ctx, cfg, runner, &out.events), out)
}

fn csv_cfg(env: &Env, rows: &[String]) -> tessia::phases::proceso::RunConfig {
    let path = write_csv(env.dir.path(), &format!("{ID}.csv"), rows);
    config(&path.to_string_lossy())
}

#[test]
fn it_prepares_the_csv_once_and_streams_only_the_prepared_invoices() {
    let env = setup();
    let mut rows = pos_rows(3);
    rows.push(excluded("EVS-9", "9999"));
    let cfg = csv_cfg(&env, &rows);
    let (log, rad) = (Log::default(), batches());
    let (summary, out) = run_with(&env, &cfg, &fake_runner(&log, &rad));

    let summary = summary.expect("run");
    assert!(summary.prepared_now);
    assert_eq!((summary.seeded, summary.finished.done), (3, 3));
    for invoice in ["EVS1", "EVS2", "EVS3"] {
        assert_eq!(log.path(invoice), MAINLINE, "{invoice}");
    }
    assert!(log.path("EVS9").is_empty(), "excluded invoices stay behind");
    assert_eq!(flatten(&rad), ["EVS1", "EVS2", "EVS3"]);

    let kinds = out.kinds();
    assert_eq!(&kinds[..3], ["convenio_start", "convenio", "run_start"]);
    assert_eq!(kinds.last().map(String::as_str), Some("run_end"));
    let convenio = &out.of("convenio")[0];
    assert_eq!(convenio.status, "OK");
    assert!(
        convenio
            .reason
            .starts_with("prepared=3 excluded=1 manual_review=0 lines=4"),
        "{}",
        convenio.reason
    );
    // The JSON stream parses line by line.
    for line in out.out.text().lines() {
        serde_json::from_str::<serde_json::Value>(line).expect("json line");
    }
}

#[test]
fn a_prepared_cargue_is_not_prepared_again() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(2));
    let (log, rad) = (Log::default(), batches());
    run_with(&env, &cfg, &fake_runner(&log, &rad))
        .0
        .expect("first");
    let versions = |env: &Env| -> Vec<i64> {
        let seq = env
            .ctx
            .dataset()
            .current_cargue(ID)
            .expect("seq")
            .expect("some");
        let mut v: Vec<i64> = env
            .ctx
            .dataset()
            .read_invoices(seq, None)
            .expect("invoices")
            .iter()
            .map(|i| i.version)
            .collect();
        v.sort_unstable();
        v
    };
    assert_eq!(versions(&env), [0, 0]);

    // A second machine (or a cron) on the same cargue by id, and again by CSV path.
    for source in [ID.to_owned(), cfg.source.clone()] {
        let again = config(&source);
        let (summary, out) = run_with(&env, &again, &fake_runner(&log, &rad));
        assert!(!summary.expect("rerun").prepared_now);
        assert!(out.of("convenio_start").is_empty());
        assert_eq!(versions(&env), [0, 0], "the dataset was not replaced");
    }
    let states = env
        .ctx
        .proceso()
        .load(Phase::Convenio, None)
        .expect("states");
    assert!(
        states
            .values()
            .all(|s| s.intentos == 1 && s.state == PhaseState::Done)
    );
}

#[test]
fn repreparar_forces_a_new_preparation() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(2));
    let (log, rad) = (Log::default(), batches());
    run_with(&env, &cfg, &fake_runner(&log, &rad))
        .0
        .expect("first");
    let mut again = config(ID);
    again.reprepare = true;
    let (summary, _) = run_with(&env, &again, &fake_runner(&log, &rad));
    assert!(summary.expect("rerun").prepared_now);
    let seq = env
        .ctx
        .dataset()
        .current_cargue(ID)
        .expect("seq")
        .expect("some");
    let versions: Vec<i64> = env
        .ctx
        .dataset()
        .read_invoices(seq, None)
        .expect("invoices")
        .iter()
        .map(|i| i.version)
        .collect();
    assert_eq!(versions, [1, 1]);
}

#[test]
fn two_machines_starting_together_prepare_the_cargue_only_once() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(3));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let prepared: Vec<bool> = std::thread::scope(|scope| {
        let handles: Vec<_> = ["machine-a", "machine-b"]
            .into_iter()
            .map(|owner| {
                let ctx = env.ctx.clone().with_owner(owner);
                let (cfg, barrier) = (cfg.clone(), barrier.clone());
                scope.spawn(move || {
                    let (log, rad) = (Log::default(), batches());
                    let out = capture(Mode::Json);
                    barrier.wait();
                    execute(&ctx, &cfg, &fake_runner(&log, &rad), &out.events)
                        .expect("run")
                        .prepared_now
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("join"))
            .collect()
    });
    assert_eq!(prepared.iter().filter(|p| **p).count(), 1, "{prepared:?}");
    let seq = env
        .ctx
        .dataset()
        .current_cargue(ID)
        .expect("seq")
        .expect("some");
    let invoices = env
        .ctx
        .dataset()
        .read_invoices(seq, None)
        .expect("invoices");
    assert!(invoices.iter().all(|i| i.version == 0), "replaced twice");
}

#[test]
fn finished_invoices_are_skipped_before_the_limit() {
    let env = setup();
    let mut done = RadState::new("EVS1", Step::Crc);
    done.status = RadStatus::Confirmed;
    env.ctx.radicacion().save(&done).expect("rad");
    let mut cfg = csv_cfg(&env, &pos_rows(5));
    cfg.limit = 2;
    cfg.no_replace = true;
    let (log, rad) = (Log::default(), batches());
    let (summary, _) = run_with(&env, &cfg, &fake_runner(&log, &rad));
    assert_eq!(summary.expect("run").seeded, 2);
    assert!(log.path("EVS1").is_empty());
    assert_eq!(flatten(&rad), ["EVS2", "EVS3"]);
}

#[test]
fn invoices_restrict_the_whole_run() {
    let env = setup();
    let mut cfg = csv_cfg(&env, &pos_rows(4));
    cfg.invoices = vec!["EVS2".into(), "EVS4".into()];
    let (log, rad) = (Log::default(), batches());
    run_with(&env, &cfg, &fake_runner(&log, &rad))
        .0
        .expect("run");
    assert_eq!(flatten(&rad), ["EVS2", "EVS4"]);
    // Stored dataset, same restriction.
    let mut again = config(ID);
    again.invoices = vec!["EVS2".into(), "NOPE9".into()];
    let rad2 = batches();
    run_with(&env, &again, &fake_runner(&log, &rad2))
        .0
        .expect("rerun");
    assert_eq!(flatten(&rad2), ["EVS2"]);
}

#[test]
fn from_to_and_skip_select_the_phases_and_the_gate() {
    let env = setup();
    let mut cfg = csv_cfg(&env, &pos_rows(2));
    cfg.active =
        steps::select(Some("fev"), Some("rips"), &["derechos".to_owned()]).expect("select");
    let (log, rad) = (Log::default(), batches());
    run_with(&env, &cfg, &fake_runner(&log, &rad))
        .0
        .expect("run");
    assert_eq!(log.path("EVS1"), ["fev", "rips"]);
    assert!(flatten(&rad).is_empty(), "radicacion is outside --to");
}

#[test]
fn only_the_gate_runs_with_from_radicacion() {
    let env = setup();
    let mut cfg = csv_cfg(&env, &pos_rows(3));
    cfg.active = steps::select(Some("radicacion"), None, &[]).expect("select");
    let (log, rad) = (Log::default(), batches());
    let (summary, _) = run_with(&env, &cfg, &fake_runner(&log, &rad));
    assert_eq!(summary.expect("run").finished.done, 3);
    assert!(log.all().is_empty());
    assert_eq!(flatten(&rad), ["EVS1", "EVS2", "EVS3"]);
}

#[test]
fn value_differences_go_through_conciliar_and_then_derechos() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(3));
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner_with(&log, &rad, |phase| -> Decide {
        match phase {
            "fev" => Arc::new(|invoice| match invoice {
                "EVS2" => Outcome::manual_review("VALOR_DIFERENTE XML=1 convenio=2"),
                "EVS3" => {
                    Outcome::failed("XML_SIN_VALOR (no se pudo extraer el valor total del XML)")
                }
                _ => Outcome::done(),
            }),
            _ => always_done(),
        }
    });
    let (summary, out) = run_with(&env, &cfg, &runner);
    let finished = summary.expect("run").finished;
    assert_eq!(log.path("EVS1"), MAINLINE);
    assert_eq!(
        log.path("EVS2"),
        [
            "thefactory",
            "fev",
            "conciliar",
            "derechos",
            "rips",
            "cuv",
            "soporte"
        ]
    );
    assert_eq!(log.path("EVS3"), ["thefactory", "fev"]);
    assert_eq!((finished.done, finished.manual, finished.failed), (2, 1, 0));
    let nil = out
        .of("invoice_step")
        .into_iter()
        .find(|e| e.invoice == "EVS3" && e.phase == "fev")
        .expect("step");
    assert_eq!(
        (nil.status.as_str(), nil.reason.as_str()),
        ("MANUAL_REVIEW", "XML_SIN_VALOR")
    );
    assert_eq!(flatten(&rad), ["EVS1", "EVS2"]);
}

#[test]
fn a_value_difference_without_conciliar_goes_to_manual_review() {
    let env = setup();
    let mut cfg = csv_cfg(&env, &pos_rows(1));
    cfg.active = steps::select(None, None, &["conciliar".to_owned()]).expect("select");
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner_with(&log, &rad, |phase| -> Decide {
        if phase == "fev" {
            Arc::new(|_| Outcome::manual_review("VALOR_DIFERENTE XML=1 convenio=2"))
        } else {
            always_done()
        }
    });
    let (summary, _) = run_with(&env, &cfg, &runner);
    let finished = summary.expect("run").finished;
    assert_eq!((finished.manual, finished.done), (1, 0));
    assert_eq!(log.path("EVS1"), ["thefactory", "fev"]);
}

#[test]
fn a_derechos_authentication_stop_is_fatal_even_with_continue_on_error() {
    let env = setup();
    let mut cfg = csv_cfg(&env, &pos_rows(4));
    cfg.continue_on_error = true;
    for pool in ["thefactory", "fev", "derechos"] {
        cfg.workers.insert(pool, 1);
    }
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner_with(&log, &rad, |_| always_done()).with("derechos", |_, _| {
        Err(DerechosError::AuthenticationStopped {
            untouched: 1,
            detail: "401".into(),
        }
        .into())
    });
    let (summary, out) = run_with(&env, &cfg, &runner);
    let err = summary.expect_err("fatal");
    assert!(
        err.to_string().contains("credenciales o token rechazados"),
        "{err:#}"
    );
    assert!(log.phase_calls("rips") == 0, "no later phase ran");
    assert_eq!(out.of("run_end").len(), 1);
}

#[test]
fn a_missing_rips_json_fails_only_that_invoice() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(3));
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner_with(&log, &rad, |_| always_done()).with("cuv", |_, req| {
        let invoice = req.invoices.clone().unwrap_or_default().remove(0);
        if invoice == "EVS2" {
            Err(MissingRipsJsonError { invoice }.into())
        } else {
            let mut report = tessia::phases::common::PhaseReport::default();
            report.push(&invoice, Outcome::done());
            Ok(report)
        }
    });
    let (summary, _) = run_with(&env, &cfg, &runner);
    let finished = summary.expect("no abort").finished;
    assert_eq!((finished.done, finished.failed), (2, 1));
    assert!(finished.error.is_none());
    assert_eq!(flatten(&rad), ["EVS1", "EVS3"]);
}

fn medicar_down() -> Incomplete {
    Incomplete {
        manual_review: 0,
        failed: 1,
        details: vec![Detail {
            invoice: "EVS2".into(),
            code: "MEDICAR".into(),
            reason: "portal caído".into(),
            failed: true,
            unavailable: true,
        }],
    }
}

#[test]
fn a_medicar_outage_in_conciliar_fails_the_invoice_but_the_run_goes_on() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(3));
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner_with(&log, &rad, |phase| -> Decide {
        if phase == "fev" {
            Arc::new(|invoice| {
                if invoice == "EVS2" {
                    Outcome::manual_review("VALOR_DIFERENTE XML=1 convenio=2")
                } else {
                    Outcome::done()
                }
            })
        } else {
            always_done()
        }
    })
    .with("conciliar", |_, _| Err(medicar_down().into()));
    let (summary, _) = run_with(&env, &cfg, &runner);
    let err = summary.expect_err("the outage is still the run's error");
    assert!(err.to_string().contains("portal caído"), "{err:#}");
    assert_eq!(flatten(&rad), ["EVS1", "EVS3"], "the others were filed");
}

#[test]
fn a_technical_conciliar_failure_aborts_unless_continue_on_error() {
    let env = setup();
    let mut cfg = csv_cfg(&env, &pos_rows(1));
    let mut incomplete = medicar_down();
    incomplete.details[0].unavailable = false;
    let runner = {
        let (log, rad) = (Log::default(), batches());
        fake_runner_with(&log, &rad, |phase| -> Decide {
            if phase == "fev" {
                Arc::new(|_| Outcome::manual_review("VALOR_DIFERENTE XML=1 convenio=2"))
            } else {
                always_done()
            }
        })
        .with("conciliar", move |_, _| Err(incomplete.clone().into()))
    };
    let (summary, _) = run_with(&env, &cfg, &runner);
    assert!(summary.is_err());
    cfg.continue_on_error = true;
    let (summary, _) = run_with(&env, &cfg, &runner);
    assert!(summary.is_err(), "still reported");
}

#[test]
fn a_recorded_manual_review_drops_the_invoice_and_the_run_succeeds() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(3));
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner_with(&log, &rad, |phase| -> Decide {
        if phase == "derechos" {
            Arc::new(|invoice| {
                if invoice == "EVS1" {
                    Outcome::manual_review("ReasonNotFound")
                } else {
                    Outcome::done()
                }
            })
        } else {
            always_done()
        }
    });
    let (summary, out) = run_with(&env, &cfg, &runner);
    let finished = summary.expect("run").finished;
    assert_eq!((finished.done, finished.manual), (2, 1));
    assert_eq!(log.path("EVS1"), ["thefactory", "fev", "derechos"]);
    assert_eq!(out.of("run_end")[0].status, "MANUAL_REVIEW");
}

#[test]
fn radicacion_is_filed_in_batches_and_the_last_partial_one_is_flushed() {
    let env = setup();
    let mut cfg = csv_cfg(&env, &pos_rows(5));
    cfg.batch_radicacion = 2;
    let (log, rad) = (Log::default(), batches());
    run_with(&env, &cfg, &fake_runner(&log, &rad))
        .0
        .expect("run");
    let mut sizes: Vec<usize> = rad.lock().expect("rad").iter().map(Vec::len).collect();
    sizes.sort_unstable();
    assert_eq!(sizes, [1, 2, 2]);
    assert_eq!(flatten(&rad).len(), 5);
}

#[test]
fn the_interval_files_a_partial_batch_before_the_end_of_the_run() {
    let env = setup();
    let mut cfg = csv_cfg(&env, &pos_rows(3));
    cfg.radicacion_interval = std::time::Duration::from_millis(60);
    let (log, rad) = (Log::default(), batches());
    let slow = fake_runner_with(&log, &rad, |phase| -> Decide {
        if phase == "soporte" {
            Arc::new(|invoice| {
                if invoice == "EVS3" {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Outcome::done()
            })
        } else {
            always_done()
        }
    });
    run_with(&env, &cfg, &slow).0.expect("run");
    let seen = rad.lock().expect("rad").clone();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0], ["EVS1", "EVS2"]);
    assert_eq!(seen[1], ["EVS3"]);
}

#[test]
fn a_batch_waits_while_another_machine_files_the_cargue() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(2));
    let (log, rad) = (Log::default(), batches());
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = attempts.clone();
    let rad_for_fake = rad.clone();
    let runner = fake_runner(&log, &rad).with("radicacion", move |ctx, req| {
        if counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
            return Err(anyhow!(
                "el cargue {} ya lo está radicando otra máquina (m2); espere a que termine",
                req.cargue_id
            ));
        }
        recording_radicacion(rad_for_fake.clone())(ctx, req)
    });
    let (summary, out) = run_with(&env, &cfg, &runner);
    let finished = summary.expect("not an error").finished;
    assert_eq!(finished.done, 2);
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(out.of("radicacion")[0].status, "OK");
}

#[test]
fn a_batch_that_never_gets_the_lease_is_left_for_the_next_run() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(2));
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner(&log, &rad).with("radicacion", |_, req| {
        Err(anyhow!(
            "el cargue {} ya lo está radicando otra máquina (m2); espere a que termine",
            req.cargue_id
        ))
    });
    let (summary, out) = run_with(&env, &cfg, &runner);
    let finished = summary.expect("not an error").finished;
    assert_eq!((finished.skipped, finished.failed), (2, 0));
    assert_eq!(out.of("radicacion")[0].status, "SKIPPED");
}

#[test]
fn a_failed_radicacion_batch_is_the_run_error() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(2));
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner(&log, &rad).with("radicacion", |_, _| Err(anyhow!("portal 500")));
    let (summary, out) = run_with(&env, &cfg, &runner);
    let err = summary.expect_err("failed");
    assert!(err.to_string().contains("radicacion: portal 500"));
    assert_eq!(out.of("radicacion")[0].status, "FAILED");
}

#[test]
fn run_wide_alerts_are_reported_after_every_outcome_is_stored() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(2));
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner(&log, &rad).with_finish(|_| {
        Err(anyhow!(
            "SISPRO: 3 de 4 facturas fallaron al iniciar sesion"
        ))
    });
    let (summary, _) = run_with(&env, &cfg, &runner);
    let err = summary.expect_err("alert");
    assert!(err.to_string().contains("SISPRO"));
    assert_eq!(
        flatten(&rad),
        ["EVS1", "EVS2"],
        "the outcomes were already stored"
    );
}

#[test]
fn the_text_stream_shows_the_header_every_step_and_the_summary() {
    let env = setup();
    let cfg = csv_cfg(&env, &pos_rows(1));
    let (log, rad) = (Log::default(), batches());
    let out = capture(Mode::Text);
    execute(&env.ctx, &cfg, &fake_runner(&log, &rad), &out.events).expect("run");
    let text = out.out.text();
    assert!(text.contains("convenio     convenio preparar  ▸ preparando…"));
    assert!(text.contains("proceso: 1 invoice(s) entering the pipeline"));
    assert!(text.contains("EVS1         thefactory   ✓ OK"));
    assert!(text.contains("-            radicacion   ✓ OK"));
    assert!(text.contains("proceso: OK (done=1 manual=0 excluded=0 failed=0)"));
}

#[test]
fn a_cargue_that_does_not_exist_is_an_error_before_anything_runs() {
    let env = setup();
    let (log, rad) = (Log::default(), batches());
    let (summary, out) = run_with(
        &env,
        &config("NoExiste_01012026_01012026"),
        &fake_runner(&log, &rad),
    );
    assert!(summary.is_err());
    assert!(out.kinds().is_empty());
    assert!(log.all().is_empty());
}

#[test]
fn replacement_prepares_the_next_pending_invoices_when_one_stops_early() {
    let env = setup();
    // EVS3 will be excluded by the rules: the supplier must skip it.
    let mut rows = vec![
        pos("EVS-1", "1001"),
        pos("EVS-2", "1002"),
        excluded("EVS-3", "1003"),
    ];
    rows.extend((4..=6).map(|i| pos(&format!("EVS-{i}"), &format!("{}", 1000 + i))));
    let mut cfg = csv_cfg(&env, &rows);
    cfg.limit = 2;
    cfg.tuning.min_supply_chunk = 1;
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner_with(&log, &rad, |phase| -> Decide {
        if phase == "rips" {
            Arc::new(|invoice| {
                if invoice == "EVS1" {
                    Outcome::manual_review("SIN_SERVICIOS: nada")
                } else {
                    Outcome::done()
                }
            })
        } else {
            always_done()
        }
    });
    let (summary, out) = run_with(&env, &cfg, &runner);
    let finished = summary.expect("run").finished;
    assert_eq!(
        (finished.done, finished.manual, finished.replaced),
        (2, 1, 1)
    );
    assert_eq!(finished.entered, 3);
    let replaces = out.of("invoice_replace");
    assert_eq!(replaces.len(), 1);
    assert_eq!(
        replaces[0].invoice, "EVS4",
        "EVS3 is excluded, EVS4 is next"
    );
    assert_eq!(
        (replaces[0].phase.as_str(), replaces[0].reason.as_str()),
        ("rips", "replaces EVS1")
    );
    assert_eq!(flatten(&rad), ["EVS2", "EVS4"]);
    assert_eq!(log.path("EVS4"), MAINLINE);
    let more = out.of("convenio_more");
    assert!(!more.is_empty());
    assert_eq!(more.iter().map(|e| e.done).sum::<usize>(), 1);
    assert_eq!(more.iter().map(|e| e.excluded).sum::<usize>(), 1);
    // The candidates were stored: merged into the dataset, excluded one recorded.
    let seq = env
        .ctx
        .dataset()
        .current_cargue(ID)
        .expect("seq")
        .expect("some");
    let stored: Vec<String> = env
        .ctx
        .dataset()
        .read_invoices(seq, None)
        .expect("invoices")
        .into_iter()
        .map(|i| i.invoice)
        .collect();
    assert_eq!(stored, ["EVS1", "EVS2", "EVS4"]);
    let states = env
        .ctx
        .proceso()
        .load(Phase::Convenio, None)
        .expect("states");
    assert_eq!(states["EVS3"].state, PhaseState::Excluded);
    assert_eq!(states["EVS4"].state, PhaseState::Done);
    assert!(
        !states.contains_key("EVS5"),
        "the part of the pool never reached is untouched"
    );
    let end = &out.of("run_end")[0];
    assert_eq!((end.total, end.replaced), (3, 1));
}

#[test]
fn no_replace_and_a_limit_of_zero_keep_the_initial_window() {
    for (limit, no_replace) in [(2usize, true), (0, false)] {
        let env = setup();
        let mut cfg = csv_cfg(&env, &pos_rows(5));
        cfg.limit = limit;
        cfg.no_replace = no_replace;
        let (log, rad) = (Log::default(), batches());
        let runner = fake_runner_with(&log, &rad, |phase| -> Decide {
            if phase == "fev" {
                Arc::new(|invoice| {
                    if invoice == "EVS1" {
                        Outcome::failed("XML_SIN_VALOR (x)")
                    } else {
                        Outcome::done()
                    }
                })
            } else {
                always_done()
            }
        });
        let (summary, out) = run_with(&env, &cfg, &runner);
        let finished = summary.expect("run").finished;
        assert_eq!(
            finished.replaced, 0,
            "limit {limit} no_replace {no_replace}"
        );
        assert!(out.of("invoice_replace").is_empty());
        assert_eq!(out.of("run_end")[0].total, 0);
    }
}

#[test]
fn replacement_needs_a_csv_prepared_in_this_call() {
    let env = setup();
    let first = csv_cfg(&env, &pos_rows(4));
    let (log, rad) = (Log::default(), batches());
    run_with(&env, &first, &fake_runner(&log, &rad))
        .0
        .expect("prepare");
    let mut again = config(ID);
    again.limit = 2;
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner_with(&log, &rad, |phase| -> Decide {
        if phase == "fev" {
            Arc::new(|_| Outcome::failed("XML_SIN_VALOR (x)"))
        } else {
            always_done()
        }
    });
    let (summary, out) = run_with(&env, &again, &runner);
    assert_eq!(summary.expect("run").finished.replaced, 0);
    assert!(out.of("invoice_replace").is_empty());
}
