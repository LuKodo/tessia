//! The in-process job runner and the job routes, through `ApiState::handle`
//! against a throwaway Postgres. The phases are fakes (no external system is
//! contacted); `proceso run` also runs the real engine over fake phases.

#[path = "proceso_support.rs"]
mod proceso;
#[path = "api_jobs_support.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use serde_json::{Value, json};
use support::*;
use tessia::domain::dataset::test_support::{DatasetBuilder, InvoiceBuilder};
use tessia::domain::{Outcome, Phase};
use tessia::jobs::{CANCELLED_ERROR, JobRunner, NoJobRunning, StartError, StartOptions, Work};
use tessia::phases::common::PhaseRequest;
use tessia::phases::proceso::{Event, execute, steps};
use tessia::stores::dataset::test_support::seed_cargue;
use tessia::stores::proceso::SaveRow;

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
    assert!(!env.jobs.current().corriendo);
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

// ------------------------------------------------------------- /api/run

fn seed(env: &Env, id: &str) -> i64 {
    seed_cargue(&env.db.pool(), &format!("{id}.csv"), 'a')
}

fn assert_error(response: &tessia::api::Response, status: u16, message: &str) {
    assert_eq!(error(response), (status, message.to_owned()));
}

fn assert_idle(env: &Env) {
    assert!(env.calls.proceso.lock().expect("calls").is_empty());
    assert!(env.calls.download.lock().expect("calls").is_empty());
    assert!(env.calls.conciliar.lock().expect("calls").is_empty());
    assert_eq!(body(&env.get("/api/jobs/current"))["id"], "");
}

#[test]
fn run_rejects_a_bad_body_before_anything_else() {
    let env = env(Fakes::default());
    let bad = |text: &str| env.post("/api/run", text);
    let invalid = |response: tessia::api::Response| {
        let (status, message) = error(&response);
        assert_eq!(status, 400);
        assert!(message.starts_with("cuerpo invalido: "), "{message}");
        message
    };
    assert!(invalid(bad("{")).len() > "cuerpo invalido: ".len());
    assert_eq!(invalid(bad("")), "cuerpo invalido: EOF");
    invalid(bad("[]"));
    assert_eq!(
        invalid(bad(r#"{"Cargue":5}"#)),
        "cuerpo invalido: json: cannot unmarshal number into Go struct field .Cargue of type string"
    );
    assert_eq!(
        invalid(bad(r#"{"Cargue":"C1","Limit":"3"}"#)),
        "cuerpo invalido: json: cannot unmarshal string into Go struct field .Limit of type int"
    );
    let huge = format!(r#"{{"Cargue":"{}"}}"#, "x".repeat(64 << 10));
    assert_error(&bad(&huge), 413, "cuerpo demasiado grande");
    assert_idle(&env);
}

#[test]
fn run_validates_in_the_documented_order_and_never_starts_a_job_on_failure() {
    let env = env(Fakes::default());
    seed(&env, "C1");
    let run = |text: &str| env.post("/api/run", text);
    assert_error(&run(r#"{}"#), 400, "cargue invalido \"\"");
    assert_error(&run(r#"{"Cargue":"a/b"}"#), 400, "cargue invalido \"a/b\"");
    assert_error(
        &run(r#"{"Cargue":"a\\b"}"#),
        400,
        "cargue invalido \"a\\\\b\"",
    );
    assert_error(&run(r#"{"Cargue":".."}"#), 400, "cargue invalido \"..\"");
    // The id check comes before the limit, the limit before the lookup.
    assert_error(
        &run(r#"{"Cargue":"","Limit":-1}"#),
        400,
        "cargue invalido \"\"",
    );
    assert_error(
        &run(r#"{"Cargue":"NOPE","Limit":-1}"#),
        400,
        "--limit must be >= 0",
    );
    assert_error(
        &run(r#"{"Cargue":"NOPE"}"#),
        404,
        "dashboard: cargue not found: \"NOPE\"",
    );
    assert_error(
        &run(r#"{"Cargue":"C1","From":"nope"}"#),
        400,
        "unknown step \"nope\" (available: convenio, thefactory, fev, conciliar, derechos, rips, cuv, soporte, radicacion)",
    );
    assert_error(
        &run(r#"{"Cargue":"C1","From":"cuv","To":"fev"}"#),
        400,
        "--from \"cuv\" is after --to \"fev\"",
    );
    assert_idle(&env);
}

#[test]
fn run_starts_the_job_with_the_plan_seeded_and_the_config_resolved() {
    let gate = Gate::default();
    let held = gate.clone();
    let env = env(Fakes::default().proceso(move |_, _, _| {
        held.wait();
        Ok(())
    }));
    seed(&env, "C1");
    // Capitalized keys as the UI sends them, trailing data ignored like Go's decoder.
    let response = env.post(
        "/api/run",
        r#"{"Cargue":"C1","From":"fev","To":"cuv","Limit":3} trailing"#,
    );
    assert_eq!(response.status, 200);
    assert_eq!(
        response.header("Content-Type"),
        Some("application/json; charset=utf-8")
    );
    assert!(response.body.ends_with(b"\n"));
    gate.reached();
    let job = body(&response);
    assert_eq!(
        (job["id"].as_str(), job["tipo"].as_str()),
        (Some("job-1"), Some("proceso run"))
    );
    assert_eq!(
        (job["corriendo"].clone(), job["cancelable"].clone()),
        (json!(true), json!(true))
    );
    assert_eq!(
        job["terminado_en"], "0001-01-01T00:00:00Z",
        "present while running"
    );
    assert!(job.get("error").is_none());
    let log = job["salida"].as_str().unwrap_or_default();
    assert!(
        log.starts_with("Inicio ")
            && log.ends_with(
                " · proceso run · cargue C1 · fases: fev verificar → conciliar → derechos validar → rips generar → cuv generar"
            ),
        "{log}"
    );
    let cards: Vec<(&str, u64)> = job["fases"]
        .as_array()
        .expect("fases")
        .iter()
        .map(|p| {
            (
                p["fase"].as_str().unwrap_or_default(),
                p["en_curso"].as_u64().unwrap_or(9),
            )
        })
        .collect();
    assert_eq!(
        cards,
        [
            ("fev", 0),
            ("conciliar", 0),
            ("derechos", 0),
            ("rips", 0),
            ("cuv", 0)
        ],
        "every planned phase is listed, queued, in plan order"
    );
    assert_eq!(job["fases"][0]["etiqueta"], "fev verificar");

    let calls = env.calls.proceso.lock().expect("calls");
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].source.as_str(), calls[0].limit), ("C1", 3));
    assert!(calls[0].active.has("fev") && calls[0].active.has("cuv"));
    assert!(!calls[0].active.has("thefactory") && !calls[0].active.has("soporte"));
    drop(calls);
    gate.open();
    env.wait_done();
}

#[test]
fn run_accepts_the_keys_in_any_case_and_defaults_the_rest() {
    let env = env(Fakes::default());
    seed(&env, "C1");
    for text in [
        r#"{"cargue":"C1","limit":2}"#,
        r#"{"CARGUE":"C1","LIMIT":2}"#,
        r#"{"Cargue":"C1","From":null,"Limit":2}"#,
    ] {
        assert_eq!(env.post("/api/run", text).status, 200, "{text}");
        env.wait_done();
    }
    let calls = env.calls.proceso.lock().expect("calls");
    assert_eq!(calls.len(), 3);
    for cfg in calls.iter() {
        assert_eq!((cfg.source.as_str(), cfg.limit), ("C1", 2));
        assert_eq!(cfg.active, steps::select(None, None, &[]).expect("all"));
    }
}

#[test]
fn a_second_start_while_a_job_runs_is_a_409_with_its_kind_and_start_time() {
    let gate = Gate::default();
    let held = gate.clone();
    let env = env(Fakes::default().proceso(move |_, _, _| {
        held.wait();
        Ok(())
    }));
    seed(&env, "C1");
    assert_eq!(env.post("/api/run", r#"{"Cargue":"C1"}"#).status, 200);
    gate.reached();
    for (path, text) in [
        ("/api/run", r#"{"Cargue":"C1"}"#),
        (
            "/api/download",
            r#"{"Desde":"2026-10-01","Hasta":"2026-10-02"}"#,
        ),
    ] {
        let (status, message) = error(&env.post(path, text));
        assert_eq!(status, 409, "{path}");
        let rest = message.strip_prefix("ya hay un proceso en curso (proceso run, iniciado ");
        let time = rest.and_then(|r| r.strip_suffix("): espera a que termine"));
        assert!(
            time.is_some_and(|t| t.len() == 8 && t.as_bytes()[2] == b':'),
            "{message}"
        );
    }
    let retry = env.post("/api/cargues/C1/reintentar", r#"{"factura":"FEV1"}"#);
    assert_ne!(retry.status, 200);
    assert_eq!(env.calls.proceso.lock().expect("calls").len(), 1);
    gate.open();
    env.wait_done();
}

#[test]
fn a_closed_runner_answers_503_to_every_start() {
    let env = env(Fakes::default());
    seed(&env, "C1");
    env.jobs.close();
    assert_error(
        &env.post("/api/run", r#"{"Cargue":"C1"}"#),
        503,
        "job runner is shutting down",
    );
    assert_error(
        &env.post(
            "/api/download",
            r#"{"Desde":"2026-10-01","Hasta":"2026-10-02"}"#,
        ),
        503,
        "job runner is shutting down",
    );
    assert_idle(&env);
}

#[test]
fn run_over_the_real_engine_feeds_the_stats_phases_convenio_and_log() {
    let csv_dir = tempfile::tempdir().expect("dir");
    let mut rows = proceso::pos_rows(3);
    rows.push(proceso::excluded("EVS-9", "9999"));
    let csv = proceso::write_csv(csv_dir.path(), &format!("{}.csv", proceso::ID), &rows);
    let csv = csv.to_string_lossy().into_owned();
    let (log, rad) = (proceso::Log::default(), proceso::batches());
    let phases = (log.clone(), rad.clone());
    // The job runs the real orchestrator over fake phases; the CSV path
    // makes step 0 prepare the cargue inside the job.
    let env = env_with_nt(
        Fakes::default().proceso(move |ctx, cfg, events| {
            let mut run = proceso::config(&csv);
            run.limit = cfg.limit;
            run.active = cfg.active.clone();
            execute(
                ctx,
                &run,
                &proceso::fake_runner(&phases.0, &phases.1),
                events,
            )
            .map(|_| ())
        }),
        proceso::catalog(),
    );
    seed(&env, proceso::ID);
    let response = env.post("/api/run", &format!(r#"{{"Cargue":"{}"}}"#, proceso::ID));
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let done = env.wait_done();

    assert!(done.get("error").is_none(), "{done}");
    assert_eq!(done["cancelable"], false);
    assert_eq!(done["convenio"]["estado"], "ok");
    assert_eq!(
        (
            done["convenio"]["preparadas"].clone(),
            done["convenio"]["excluidas"].clone()
        ),
        (json!(3), json!(1))
    );
    let stats = &done["estadisticas"];
    assert_eq!(
        (
            stats["esperadas"].clone(),
            stats["procesadas"].clone(),
            stats["terminadas"].clone()
        ),
        (json!(3), json!(3), json!(3))
    );
    assert_eq!(
        (stats["con_errores"].clone(), stats["excluidas"].clone()),
        (json!(0), json!(1))
    );
    assert_eq!(stats["gestionadas"], 3);
    let phases: Vec<(&str, u64)> = done["fases"]
        .as_array()
        .expect("fases")
        .iter()
        .map(|p| {
            (
                p["fase"].as_str().unwrap_or_default(),
                p["terminadas"].as_u64().unwrap_or(0),
            )
        })
        .collect();
    assert_eq!(
        phases,
        [
            ("thefactory", 3),
            ("fev", 3),
            ("derechos", 3),
            ("rips", 3),
            ("cuv", 3),
            ("soporte", 3),
            ("radicacion", 3)
        ],
        "the seeded plan order; conciliar never ran and is dropped"
    );
    assert_eq!(done["radicadas"], 3);
    let lines: Vec<&str> = done["salida"].as_str().expect("salida").lines().collect();
    assert!(lines[0].starts_with("Inicio ") && lines[0].contains(" · proceso run · cargue Convenio_21102026_21102026 · fases: convenio preparar → thefactory descargar → "), "{}", lines[0]);
    assert_eq!(
        lines[1],
        "Convenio preparar: 3 preparadas · 1 excluidas · 0 revisión manual · 4 evaluadas"
    );
    assert!(
        lines[2].starts_with("Thefactory descargar: 3 ok · 0 revisión manual · 0 con errores · "),
        "{}",
        lines[2]
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("Radicacion run: 3 ok · 0 revisión manual · 0 con errores · "))
    );
    let last = lines.last().copied().unwrap_or_default();
    assert!(
        last.starts_with("Proceso terminado en ")
            && last.ends_with(": 3 terminadas · 0 con errores · 0 revisión manual · 1 excluidas"),
        "{last}"
    );
    assert_eq!(proceso::flatten(&rad), ["EVS1", "EVS2", "EVS3"]);
}

#[test]
fn a_failed_phase_shows_in_the_log_as_an_error_line() {
    let csv_dir = tempfile::tempdir().expect("dir");
    let csv = proceso::write_csv(
        csv_dir.path(),
        &format!("{}.csv", proceso::ID),
        &proceso::pos_rows(2),
    );
    let csv = csv.to_string_lossy().into_owned();
    let env = env_with_nt(
        Fakes::default().proceso(move |ctx, _, events| {
            let (log, rad) = (proceso::Log::default(), proceso::batches());
            let runner = proceso::fake_runner_with(&log, &rad, |name| {
                if name == "rips" {
                    std::sync::Arc::new(|invoice: &str| {
                        if invoice == "EVS1" {
                            Outcome::failed("RIPS_BOOM: sin datos")
                        } else {
                            Outcome::done()
                        }
                    })
                } else {
                    proceso::always_done()
                }
            });
            execute(ctx, &proceso::config(&csv), &runner, events).map(|_| ())
        }),
        proceso::catalog(),
    );
    seed(&env, proceso::ID);
    env.post("/api/run", &format!(r#"{{"Cargue":"{}"}}"#, proceso::ID));
    let done = env.wait_done();
    let output = done["salida"].as_str().expect("salida");
    assert!(
        output.contains("Rips generar: 1 ok · 0 revisión manual · 1 con errores · "),
        "{output}"
    );
    assert!(
        output.contains("\nError · EVS1 · rips generar · "),
        "{output}"
    );
    assert_eq!(done["estadisticas"]["con_errores"], 1);
}

// ------------------------------------------------------------ /api/download

#[test]
fn download_validates_the_dates_in_order() {
    let env = env(Fakes::default());
    let post = |text: &str| env.post("/api/download", text);
    assert_error(&post(r#"{}"#), 400, "--desde must be YYYY-MM-DD");
    assert_error(
        &post(r#"{"Desde":"01/10/2026","Hasta":"2026-10-02"}"#),
        400,
        "--desde must be YYYY-MM-DD",
    );
    assert_error(
        &post(r#"{"Desde":"2026-10-01"}"#),
        400,
        "--hasta must be YYYY-MM-DD",
    );
    assert_error(
        &post(r#"{"Desde":"2026-10-01","Hasta":"2026-13-01"}"#),
        400,
        "--hasta must be YYYY-MM-DD",
    );
    assert_error(
        &post(r#"{"Desde":"2026-10-05","Hasta":"2026-10-01"}"#),
        400,
        "--hasta is before --desde",
    );
    let (status, message) = error(&post("not json"));
    assert_eq!(status, 400);
    assert!(message.starts_with("cuerpo invalido: "), "{message}");
    assert_error(&post(""), 400, "cuerpo invalido: EOF");
    assert_idle(&env);
}

#[test]
fn download_refuses_a_range_that_already_has_a_cargue() {
    let env = env(Fakes::default());
    seed(&env, "Convenio_01102026_05102026");
    let existing = env.post(
        "/api/download",
        r#"{"Desde":"2026-10-01","Hasta":"2026-10-05"}"#,
    );
    assert_error(
        &existing,
        409,
        "Ya existe un cargue para esas fechas; no se reemplazó.",
    );
    assert_idle(&env);
    // Another range is free.
    assert_eq!(
        env.post(
            "/api/download",
            r#"{"Desde":"2026-10-01","Hasta":"2026-10-06"}"#
        )
        .status,
        200
    );
    env.wait_done();
}

#[test]
fn download_starts_convenio_descargar_for_the_range() {
    let env = env(Fakes::default());
    let response = env.post(
        "/api/download",
        r#"{"desde":" 2026-10-01 ","HASTA":"2026-10-05"}"#,
    );
    assert_eq!(response.status, 200);
    let job = body(&response);
    assert_eq!(
        (job["id"].as_str(), job["tipo"].as_str()),
        (Some("job-1"), Some("convenio descargar"))
    );
    let done = env.wait_done();
    assert!(done.get("error").is_none(), "{done}");
    let dates = env.calls.download.lock().expect("calls").clone();
    assert_eq!(dates.len(), 1);
    assert_eq!(dates[0].0.to_string(), "2026-10-01");
    assert_eq!(dates[0].1.to_string(), "2026-10-05");
    let log = done["salida"].as_str().expect("salida");
    assert!(
        log.contains(" · convenio descargar") && !log.contains("cargue "),
        "{log}"
    );
    assert!(log.contains("\nConvenio descargar terminado en "), "{log}");
}

#[test]
fn a_failed_download_is_the_job_error() {
    let env = env(Fakes::default().download(|_, _| Err(anyhow!("medicar: sin conexión"))));
    env.post(
        "/api/download",
        r#"{"Desde":"2026-10-01","Hasta":"2026-10-05"}"#,
    );
    let done = env.wait_done();
    assert_eq!(done["error"], "medicar: sin conexión");
    let log = done["salida"].as_str().expect("salida");
    assert!(
        log.contains("Convenio descargar con error tras ")
            && log.ends_with(": medicar: sin conexión"),
        "{log}"
    );
}

// ---------------------------------------------------------- reintentar

/// A cargue with a prepared dataset of `invoices` (and its `cargues` row).
fn prepared(env: &Env, id: &str, invoices: &[&str]) {
    let seq = seed(env, id);
    let mut builder = DatasetBuilder::new(seq).load(id);
    for invoice in invoices {
        builder = builder.invoice(InvoiceBuilder::new(invoice).line("10"));
    }
    env.ctx
        .dataset()
        .replace(&builder.build(), false)
        .expect("dataset");
}

fn state(env: &Env, invoice: &str, phase: Phase, outcome: Outcome) {
    env.ctx
        .proceso()
        .save(&SaveRow {
            invoice: invoice.to_owned(),
            phase,
            outcome,
            intentos: 1,
            cargue_id: Some("C1".to_owned()),
        })
        .expect("state");
}

fn value_difference(env: &Env, invoice: &str) {
    state(
        env,
        invoice,
        Phase::FevValor,
        Outcome::manual_review("VALOR_DIFERENTE: xml 120 vs convenio 100"),
    );
}

const NOT_ELIGIBLE: &str = "la factura no tiene una revisión manual VALOR_DIFERENTE elegible";

#[test]
fn reintentar_rejects_a_bad_body() {
    let env = env(Fakes::default());
    prepared(&env, "C1", &["FEV1"]);
    let post = |text: &str| env.post("/api/cargues/C1/reintentar", text);
    for text in [
        "",
        "{",
        "[]",
        "1",
        r#"{"factura":1}"#,
        r#"{"factura":["a"]}"#,
        r#"{"factura":"FEV1","x":1}"#,
        r#"{"factura":"FEV1"} {}"#,
        r#"{"factura":"FEV1"} x"#,
    ] {
        assert_error(&post(text), 400, "cuerpo JSON inválido");
    }
    let long = format!(r#"{{"factura":"{}"}}"#, "F".repeat(4096));
    assert_error(&post(&long), 400, "cuerpo JSON inválido");
    for text in ["null", "{}", r#"{"factura":null}"#, r#"{"factura":"  "}"#] {
        assert_error(&post(text), 400, "factura requerida");
    }
    assert_idle(&env);
}

#[test]
fn reintentar_answers_404_for_an_unknown_or_invalid_cargue() {
    let env = env(Fakes::default());
    let body = r#"{"factura":"FEV1"}"#;
    assert_error(
        &env.post("/api/cargues/NOPE/reintentar", body),
        404,
        "dashboard: cargue not found: \"NOPE\"",
    );
    assert_error(
        &env.post("/api/cargues/../reintentar", body),
        404,
        "dashboard: cargue not found",
    );
    assert_error(
        &env.post("/api/cargues/a%5Cb/reintentar", body),
        404,
        "dashboard: cargue not found: \"a%5Cb\"",
    );
    assert_idle(&env);
}

#[test]
fn reintentar_requires_a_valor_diferente_manual_review_of_fev_valor() {
    let env = env(Fakes::default());
    prepared(&env, "C1", &["FEV1", "FEV2", "FEV3", "FEV4", "FEV5"]);
    let retry = |invoice: &str| {
        env.post(
            "/api/cargues/C1/reintentar",
            &format!(r#"{{"factura":"{invoice}"}}"#),
        )
    };
    // No state at all.
    assert_error(&retry("FEV1"), 409, NOT_ELIGIBLE);
    // Another phase, another state, another reason.
    state(
        &env,
        "FEV1",
        Phase::Conciliacion,
        Outcome::manual_review("VALOR_DIFERENTE: x"),
    );
    state(&env, "FEV2", Phase::FevValor, Outcome::done());
    state(
        &env,
        "FEV3",
        Phase::FevValor,
        Outcome::manual_review("DIFERENCIA_PERSISTE: x"),
    );
    state(
        &env,
        "FEV4",
        Phase::FevValor,
        Outcome::failed("VALOR_DIFERENTE: x"),
    );
    for invoice in ["FEV1", "FEV2", "FEV3", "FEV4"] {
        assert_error(&retry(invoice), 409, NOT_ELIGIBLE);
    }
    // Eligible reason but not an invoice of the cargue's dataset.
    value_difference(&env, "FEV9");
    assert_error(&retry("FEV9"), 409, NOT_ELIGIBLE);
    // Eligible, but the cargue has no dataset yet.
    seed(&env, "C2");
    assert_error(
        &env.post("/api/cargues/C2/reintentar", r#"{"factura":"FEV5"}"#),
        409,
        NOT_ELIGIBLE,
    );
    assert_idle(&env);
}

#[test]
fn reintentar_starts_conciliar_for_the_one_invoice() {
    let env = env(Fakes::default());
    prepared(&env, "C1", &["FEV1", "EVS22"]);
    value_difference(&env, "FEV1");
    // The reason may start with the code without a colon, and the invoice
    // is normalized like everywhere else (trim, dashes, upper case).
    state(
        &env,
        "EVS22",
        Phase::FevValor,
        Outcome::manual_review("VALOR_DIFERENTE xml 5 vs convenio 4"),
    );
    let response = env.post("/api/cargues/C1/reintentar", r#"{"Factura":"  fev-1 "}"#);
    assert_eq!(response.status, 200);
    let job = body(&response);
    assert_eq!(
        (job["id"].as_str(), job["tipo"].as_str()),
        (Some("job-1"), Some("conciliar invoice FEV1"))
    );
    assert_eq!(
        job["salida"]
            .as_str()
            .map(|s| s.contains(" · conciliar invoice FEV1")),
        Some(true)
    );
    let done = env.wait_done();
    assert!(done.get("error").is_none(), "{done}");
    let log = done["salida"].as_str().expect("salida");
    assert!(
        log.contains("Conciliar invoice FEV1 terminado en "),
        "{log}"
    );
    assert_eq!(
        env.post("/api/cargues/C1/reintentar", r#"{"factura":"EVS-22"}"#)
            .status,
        200
    );
    env.wait_done();
    let calls = env.calls.conciliar.lock().expect("calls");
    let asked: Vec<(String, Option<Vec<String>>, Option<usize>)> = calls
        .iter()
        .map(|r: &PhaseRequest| (r.cargue_id.clone(), r.invoices.clone(), r.workers))
        .collect();
    assert_eq!(
        asked,
        [
            ("C1".to_owned(), Some(vec!["FEV1".to_owned()]), None),
            ("C1".to_owned(), Some(vec!["EVS22".to_owned()]), None)
        ]
    );
}

#[test]
fn a_permanent_block_after_the_check_supersedes_the_retry() {
    let env = env(Fakes::default());
    prepared(&env, "C1", &["FEV1", "FEV2", "FEV3", "FEV4"]);
    let retry = |invoice: &str| {
        env.post(
            "/api/cargues/C1/reintentar",
            &format!(r#"{{"factura":"{invoice}"}}"#),
        )
    };
    // conciliar ruled FEV1 out after fev_valor: nothing left to reconcile.
    value_difference(&env, "FEV1");
    state(
        &env,
        "FEV1",
        Phase::Conciliacion,
        Outcome::excluded("BLOQUEADA_TARIFA: un precio supera la nota técnica"),
    );
    value_difference(&env, "FEV2");
    state(
        &env,
        "FEV2",
        Phase::Conciliacion,
        Outcome::excluded("BLOQUEADA_MONTO: supera el máximo"),
    );
    assert_error(&retry("FEV1"), 409, NOT_ELIGIBLE);
    assert_error(&retry("FEV2"), 409, NOT_ELIGIBLE);
    // The block is older than a fresh fev_valor check: eligible again.
    state(
        &env,
        "FEV3",
        Phase::Conciliacion,
        Outcome::excluded("BLOQUEADA_TARIFA: x"),
    );
    value_difference(&env, "FEV3");
    // A non-permanent conciliar result does not block the retry.
    value_difference(&env, "FEV4");
    state(
        &env,
        "FEV4",
        Phase::Conciliacion,
        Outcome::manual_review("DIFERENCIA_PERSISTE: x"),
    );
    assert_eq!(retry("FEV3").status, 200);
    env.wait_done();
    assert_eq!(retry("FEV4").status, 200);
    env.wait_done();
}

#[test]
fn reintentar_answers_503_when_the_state_cannot_be_read() {
    let env = env(Fakes::default());
    prepared(&env, "C1", &["FEV1"]);
    value_difference(&env, "FEV1");
    {
        use diesel::RunQueryDsl;
        diesel::sql_query("DROP TABLE proceso_estado")
            .execute(&mut *env.db.conn())
            .expect("drop");
    }
    let response = env.post("/api/cargues/C1/reintentar", r#"{"factura":"FEV1"}"#);
    assert_error(
        &response,
        503,
        "no se pudo verificar el estado persistido de la factura",
    );
    assert!(
        !String::from_utf8_lossy(&response.body).contains("proceso_estado"),
        "no raw database error"
    );
    assert_idle(&env);
}

// ------------------------------------------- current, history and cancel

#[test]
fn history_is_null_before_any_job_and_lists_the_finished_ones_newest_first() {
    let env = env(Fakes::default().proceso(|_, cfg, _| {
        if cfg.limit == 2 {
            Err(anyhow!("fallo en la fase"))
        } else {
            Ok(())
        }
    }));
    seed(&env, "C1");
    let empty = env.get("/api/jobs/history");
    assert_eq!(empty.status, 200);
    assert_eq!(body(&empty), Value::Null);
    let idle = body(&env.get("/api/jobs/current"));
    assert_eq!(idle["instancia_servidor"], env.jobs.instance_id());
    assert_eq!(
        (idle["id"].clone(), idle["corriendo"].clone()),
        (json!(""), json!(false))
    );

    for limit in [1, 2] {
        env.post("/api/run", &format!(r#"{{"Cargue":"C1","Limit":{limit}}}"#));
        env.wait_done();
    }
    let history = body(&env.get("/api/jobs/history"));
    let entries = history.as_array().expect("array");
    assert_eq!(entries.len(), 2);
    assert_eq!(
        (entries[0]["id"].clone(), entries[1]["id"].clone()),
        (json!("job-2"), json!("job-1"))
    );
    assert_eq!(entries[0]["error"], "fallo en la fase");
    assert!(entries[1].get("error").is_none());
    for entry in entries {
        assert_eq!(entry["tipo"], "proceso run");
        assert!(entry["iniciado_en"].is_string() && entry["terminado_en"].is_string());
        assert!(
            entry["salida_final"]
                .as_str()
                .is_some_and(|s| s.starts_with("Inicio "))
        );
    }
    assert!(
        entries[0]["salida_final"]
            .as_str()
            .is_some_and(|s| s.contains("Proceso con error tras "))
    );
}

#[test]
fn cancel_stops_the_running_job_and_a_second_cancel_is_a_409() {
    let gate = Gate::default();
    let held = gate.clone();
    let env = env(Fakes::default().proceso(move |_, _, _| {
        held.wait();
        Err(anyhow!("interrumpido"))
    }));
    seed(&env, "C1");
    assert_error(
        &env.post("/api/jobs/cancel", ""),
        409,
        "no hay ningun proceso en curso",
    );
    env.post("/api/run", r#"{"Cargue":"C1"}"#);
    gate.reached();
    let cancelled = env.post("/api/jobs/cancel", "");
    assert_eq!(cancelled.status, 200);
    assert_eq!(body(&cancelled), json!({"ok": true}));
    let done = env.wait_done();
    assert_eq!(done["error"], "cancelado por el operador");
    assert_eq!(done["cancelable"], false);
    assert!(
        done["salida"]
            .as_str()
            .is_some_and(|s| s.contains("\nProceso cancelado por el operador tras "))
    );
    assert_error(
        &env.post("/api/jobs/cancel", ""),
        409,
        "no hay ningun proceso en curso",
    );
    let history = body(&env.get("/api/jobs/history"));
    assert_eq!(history[0]["error"], "cancelado por el operador");
}

#[test]
fn the_job_routes_only_answer_their_methods() {
    let env = env(Fakes::default());
    for (method, path, allow) in [
        ("GET", "/api/run", "POST"),
        ("GET", "/api/download", "POST"),
        ("GET", "/api/jobs/cancel", "POST"),
        ("POST", "/api/jobs/current", "GET"),
        ("POST", "/api/jobs/history", "GET"),
        ("GET", "/api/cargues/C1/reintentar", "POST"),
    ] {
        let response = env.state.handle(tessia::api::Request::new(method, path));
        assert_eq!(response.status, 405, "{method} {path}");
        assert!(response.header("Allow").is_some_and(|a| a.contains(allow)));
    }
}
