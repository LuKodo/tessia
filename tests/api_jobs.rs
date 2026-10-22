//! The in-process job runner and the job routes, through `ApiState::handle`
//! against a throwaway Postgres. The phases are fakes (no external system is
//! contacted); `proceso run` also runs the real engine over fake phases.

#[path = "api_jobs_support.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use serde_json::{Value, json};
use support::*;
use tessia::jobs::{CANCELLED_ERROR, JobRunner, NoJobRunning, StartError, StartOptions, Work};
use tessia::phases::proceso::Event;

fn work(run: impl FnOnce() -> anyhow::Result<()> + Send + 'static) -> Work {
    Box::new(move |_, _| run())
}

fn start(
    env: &Env,
    kind: &str,
    run: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
) -> Value {
    let snapshot = env
        .jobs
        .start(kind, StartOptions::default(), work(run))
        .expect("start");
    serde_json::to_value(snapshot).expect("json")
}

// ------------------------------------------------------------ the runner

#[test]
fn one_job_runs_at_a_time_and_a_finished_one_makes_room() {
    let env = env(Fakes::default());
    let gate = Gate::default();
    let held = gate.clone();
    let first = start(&env, "proceso run", move || {
        held.wait();
        Ok(())
    });
    gate.reached();
    assert_eq!(first["id"], "job-1");
    assert_eq!(
        (first["corriendo"].clone(), first["cancelable"].clone()),
        (json!(true), json!(true))
    );

    let refused = env
        .jobs
        .start(
            "convenio descargar",
            StartOptions::default(),
            work(|| Ok(())),
        )
        .expect_err("second");
    let StartError::Running { kind, started } = &refused else {
        panic!("expected Running, got {refused:?}");
    };
    assert_eq!(kind, "proceso run");
    assert_eq!(started.len(), 8, "HH:MM:SS: {started}");
    assert_eq!(
        refused.to_string(),
        format!(
            "ya hay un proceso en curso (proceso run, iniciado {started}): espera a que termine"
        )
    );

    gate.open();
    let done = env.wait_done();
    assert_eq!(done["id"], "job-1");
    assert!(
        done["terminado_en"]
            .as_str()
            .is_some_and(|t| !t.starts_with("0001"))
    );
    let second = start(&env, "convenio descargar", || Ok(()));
    assert_eq!(second["id"], "job-2");
    env.wait_done();
}

#[test]
fn a_failure_is_the_job_error_and_a_panic_becomes_panic_value() {
    let env = env(Fakes::default());
    start(&env, "proceso run", || {
        Err(anyhow!("la base cayó").context("fev"))
    });
    assert_eq!(env.wait_done()["error"], "fev: la base cayó");

    start(&env, "proceso run", || panic!("boom"));
    assert_eq!(env.wait_done()["error"], "panic: boom");

    start(&env, "proceso run", || {
        let code = 7;
        panic!("código {code}")
    });
    assert_eq!(env.wait_done()["error"], "panic: código 7");

    let history = env.jobs.history();
    let errors: Vec<&str> = history.iter().map(|e| e.error.as_str()).collect();
    assert_eq!(
        errors,
        ["panic: código 7", "panic: boom", "fev: la base cayó"]
    );
    // The runner is still usable after a panic.
    start(&env, "proceso run", || Ok(()));
    assert_eq!(env.wait_done()["error"], Value::Null);
}

#[test]
fn cancel_raises_the_shared_flag_ends_the_job_as_cancelled_and_lowers_it() {
    let env = env(Fakes::default());
    assert_eq!(env.jobs.cancel(), Err(NoJobRunning));
    assert_eq!(NoJobRunning.to_string(), "no hay ningun proceso en curso");

    let gate = Gate::default();
    let held = gate.clone();
    start(&env, "proceso run", move || {
        held.wait(); // like the phases: stops when `shutdown::requested()`
        assert!(tessia::shutdown::requested(), "only a cancel releases it");
        Err(anyhow!("interrumpido"))
    });
    gate.reached();
    assert!(!tessia::shutdown::requested());
    env.jobs.cancel().expect("cancel");
    let done = env.wait_done();
    assert_eq!(done["error"], CANCELLED_ERROR);
    assert_eq!(CANCELLED_ERROR, "cancelado por el operador");
    assert!(
        done["salida"]
            .as_str()
            .unwrap_or_default()
            .contains("Proceso cancelado por el operador tras "),
        "{done}"
    );
    assert!(
        !tessia::shutdown::requested(),
        "the job cancel is lowered when the job ends"
    );
    assert_eq!(
        env.jobs.cancel(),
        Err(NoJobRunning),
        "nothing left to cancel"
    );

    // The next job does not start cancelled.
    start(&env, "proceso run", || {
        assert!(!tessia::shutdown::requested());
        Ok(())
    });
    assert_eq!(env.wait_done()["error"], Value::Null);
}

#[test]
fn a_cancelled_job_that_still_succeeds_has_no_error() {
    let env = env(Fakes::default());
    let gate = Gate::default();
    let held = gate.clone();
    start(&env, "proceso run", move || {
        held.wait();
        Ok(())
    });
    gate.reached();
    env.jobs.cancel().expect("cancel");
    assert_eq!(env.wait_done()["error"], Value::Null);
}

#[test]
fn close_rejects_new_starts_cancels_the_running_job_and_waits_for_it() {
    let env = env(Fakes::default());
    let gate = Gate::default();
    let held = gate.clone();
    start(&env, "proceso run", move || {
        held.wait();
        std::thread::sleep(Duration::from_millis(50)); // cleanup after the cancel
        Err(anyhow!("interrumpido"))
    });
    gate.reached();
    env.jobs.close();
    // `close` returned only after the job ended and was recorded.
    let history = env.jobs.history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].error, CANCELLED_ERROR);
    assert_eq!(env.jobs.current().corriendo, false);
    let refused = env
        .jobs
        .start("proceso run", StartOptions::default(), work(|| Ok(())))
        .expect_err("stopping");
    assert_eq!(refused, StartError::Stopping);
    assert_eq!(refused.to_string(), "job runner is shutting down");
    env.jobs.close(); // idempotent
}

#[test]
fn close_without_a_job_returns_at_once() {
    let env = env(Fakes::default());
    env.jobs.close();
    assert!(!tessia::shutdown::requested());
}

#[test]
fn finished_jobs_are_persisted_and_a_new_runner_continues_the_ids() {
    let env = env(Fakes::default());
    for n in 1..=3 {
        start(&env, "proceso run", move || {
            if n == 2 {
                Err(anyhow!("fallo {n}"))
            } else {
                Ok(())
            }
        });
        env.wait_done();
    }
    let path = env.history_dir.path().join(".tessia-serve-jobs.jsonl");
    let lines: Vec<Value> = std::fs::read_to_string(&path)
        .expect("history file")
        .lines()
        .map(|l| serde_json::from_str(l).expect("json line"))
        .collect();
    let ids: Vec<&str> = lines
        .iter()
        .map(|l| l["id"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        ids,
        ["job-1", "job-2", "job-3"],
        "append-only, oldest first"
    );
    assert_eq!(lines[1]["error"], "fallo 2");
    assert!(lines[0].get("error").is_none(), "an empty error is omitted");
    assert_eq!(lines[0]["tipo"], "proceso run");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // A restarted process loads them newest first and continues after job-3.
    let calls = Arc::new(Calls::default());
    let restarted = runner(&env.ctx, &env.history_dir, Fakes::default(), &calls);
    let loaded: Vec<String> = restarted.history().into_iter().map(|e| e.id).collect();
    assert_eq!(loaded, ["job-3", "job-2", "job-1"]);
    assert_ne!(restarted.instance_id(), env.jobs.instance_id());
    let first = restarted
        .start("proceso run", StartOptions::default(), work(|| Ok(())))
        .expect("start");
    assert_eq!(first.id, "job-4");
    restarted.close();
}

#[test]
fn a_runner_without_a_history_directory_keeps_memory_only() {
    let env = env(Fakes::default());
    let memory = JobRunner::with_history_dir(env.ctx.clone(), None);
    memory
        .start("proceso run", StartOptions::default(), work(|| Ok(())))
        .expect("start");
    memory.close();
    assert_eq!(memory.history().len(), 1);
    assert!(!memory.instance_id().is_empty());
}

#[test]
fn the_idle_snapshot_carries_only_the_instance() {
    let env = env(Fakes::default());
    let snapshot = serde_json::to_value(env.jobs.current()).expect("json");
    assert_eq!(
        snapshot,
        json!({
            "id": "", "tipo": "", "corriendo": false, "cancelable": false,
            "iniciado_en": "0001-01-01T00:00:00Z", "terminado_en": "0001-01-01T00:00:00Z",
            "salida": "", "instancia_servidor": env.jobs.instance_id()
        })
    );
    assert!(env.jobs.history().is_empty());
}

#[test]
fn the_events_of_a_job_feed_its_snapshot_while_it_runs() {
    let env = env(Fakes::default());
    let gate = Gate::default();
    let held = gate.clone();
    let work: Work = Box::new(move |_, events| {
        events.emit(Event::new("convenio_start").step("convenio preparar"));
        let mut done = Event::new("convenio")
            .step("convenio preparar")
            .status("OK");
        (done.done, done.excluded, done.manual, done.evaluated) = (4, 1, 1, 6);
        events.emit(done);
        let mut start = Event::new("run_start");
        start.total = 4;
        events.emit(start);
        events.emit(
            Event::new("invoice_enter")
                .invoice("FEV1")
                .phase("fev", "fev verificar"),
        );
        held.wait();
        Ok(())
    });
    env.jobs
        .start("proceso run", StartOptions::default(), work)
        .expect("start");
    gate.reached();
    let live = serde_json::to_value(env.jobs.current()).expect("json");
    assert_eq!(live["convenio"]["estado"], "ok");
    assert_eq!(live["convenio"]["preparadas"], 4);
    assert_eq!(live["estadisticas"]["esperadas"], 4);
    assert_eq!(live["fases"][0]["fase"], "fev");
    assert_eq!(live["fases"][0]["en_curso"], 1);
    assert!(
        live["salida"].as_str().unwrap_or_default().ends_with(
            "Convenio preparar: 4 preparadas · 1 excluidas · 1 revisión manual · 6 evaluadas"
        ),
        "{live}"
    );
    gate.open();
    env.wait_done();
}
