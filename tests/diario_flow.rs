//! `facturacion diario`: previous-day download -> prepare -> `proceso run`,
//! idempotent and safe next to other machines. The download is a fake that
//! registers the CSV like `convenio descargar` does; no external system.

#[path = "proceso_support.rs"]
mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use chrono::NaiveDate;
use support::*;
use tessia::context::AppContext;
use tessia::domain::Outcome;
use tessia::phases::convenio_descargar::{DownloadOutcome, DownloadStatus, file_name};
use tessia::phases::convenio_preparar::source::load_source;
use tessia::phases::proceso::Mode;
use tessia::phases::proceso::diario::{Csv, cargue_of, run_day};

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 21).expect("date")
}

/// Registers the CSV of `day` (rows `invoices`) as the real download would.
fn register(env: &Env, ctx: &AppContext, invoices: usize) -> DownloadOutcome {
    let name = file_name(day(), day());
    let path = write_csv(env.dir.path(), &name, &pos_rows(invoices));
    let loaded = load_source(ctx, &path.to_string_lossy()).expect("register");
    DownloadOutcome {
        cargue_id: loaded.info.id,
        file_name: name,
        object_key: String::new(),
        sha256: loaded.sha256,
        size: loaded.bytes.len() as u64,
        status: DownloadStatus::Created,
    }
}

fn first_run(env: &Env, invoices: usize) -> (Log, usize) {
    let downloads = AtomicUsize::new(0);
    let download = |ctx: &AppContext, _: NaiveDate| {
        downloads.fetch_add(1, Ordering::SeqCst);
        Ok(register(env, ctx, invoices))
    };
    let (log, rad) = (Log::default(), batches());
    let out = capture(Mode::Json);
    let summary = run_day(
        &env.ctx,
        day(),
        &config("ignored"),
        false,
        &download,
        &fake_runner(&log, &rad),
        &out.events,
    )
    .expect("diario");
    assert_eq!(summary.csv, Csv::Downloaded);
    assert_eq!(summary.cargue_id, ID);
    assert!(summary.run.prepared_now);
    assert_eq!(summary.run.finished.done, invoices);
    (log, downloads.load(Ordering::SeqCst))
}

#[test]
fn the_day_is_downloaded_prepared_and_processed_end_to_end() {
    let env = setup();
    let (log, downloads) = first_run(&env, 3);
    assert_eq!(downloads, 1);
    for invoice in ["EVS1", "EVS2", "EVS3"] {
        assert_eq!(log.path(invoice).len(), 6, "{invoice}");
    }
}

#[test]
fn running_it_again_reuses_the_csv_and_the_dataset() {
    let env = setup();
    first_run(&env, 3);
    let seq = env
        .ctx
        .dataset()
        .current_cargue(ID)
        .expect("seq")
        .expect("some");
    let downloads = AtomicUsize::new(0);
    let download = |_: &AppContext, _: NaiveDate| {
        downloads.fetch_add(1, Ordering::SeqCst);
        Err(anyhow!("must not download"))
    };
    let (log, rad) = (Log::default(), batches());
    let out = capture(Mode::Json);
    let summary = run_day(
        &env.ctx,
        day(),
        &config("ignored"),
        false,
        &download,
        &fake_runner(&log, &rad),
        &out.events,
    )
    .expect("second run");
    assert_eq!(downloads.load(Ordering::SeqCst), 0);
    assert_eq!(summary.csv, Csv::Reused);
    assert!(!summary.run.prepared_now);
    assert!(out.of("convenio_start").is_empty());
    // The invoices were not reset.
    assert_eq!(
        env.ctx.dataset().current_cargue(ID).expect("seq"),
        Some(seq)
    );
    let invoices = env
        .ctx
        .dataset()
        .read_invoices(seq, None)
        .expect("invoices");
    assert!(invoices.iter().all(|i| i.version == 0));
    let lines = summary.lines();
    assert!(
        lines[0].contains("CSV reutilizado, ya estaba preparado"),
        "{lines:?}"
    );
}

#[test]
fn redescargar_downloads_again_and_an_unchanged_csv_changes_nothing() {
    let env = setup();
    first_run(&env, 2);
    let downloads = AtomicUsize::new(0);
    let download = |ctx: &AppContext, _: NaiveDate| {
        downloads.fetch_add(1, Ordering::SeqCst);
        Ok(register(&env, ctx, 2))
    };
    let (log, rad) = (Log::default(), batches());
    let out = capture(Mode::Json);
    let summary = run_day(
        &env.ctx,
        day(),
        &config("ignored"),
        true,
        &download,
        &fake_runner(&log, &rad),
        &out.events,
    )
    .expect("forced");
    assert_eq!(downloads.load(Ordering::SeqCst), 1);
    assert_eq!(summary.csv, Csv::Downloaded);
    assert!(
        !summary.run.prepared_now,
        "same bytes: the dataset is still current"
    );
}

#[test]
fn it_waits_for_another_machine_that_is_downloading_the_same_day() {
    let env = setup();
    let (id, _) = cargue_of(day());
    let lease = format!("convenio_descargar:{id}");
    assert!(
        env.ctx
            .leases()
            .acquire(&lease, "other-machine", Duration::from_secs(60))
            .expect("lease")
    );
    let attempts = AtomicUsize::new(0);
    let download = |_: &AppContext, _: NaiveDate| {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err(anyhow!("otra máquina ya está descargando el cargue {id}"))
    };
    let (log, rad) = (Log::default(), batches());
    let out = capture(Mode::Json);
    let summary = std::thread::scope(|scope| {
        let other = env.ctx.clone().with_owner("other-machine");
        let (env, lease) = (&env, &lease);
        scope.spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            register(env, &other, 2);
            other
                .leases()
                .release(lease, "other-machine")
                .expect("release");
        });
        run_day(
            &env.ctx,
            day(),
            &config("ignored"),
            false,
            &download,
            &fake_runner(&log, &rad),
            &out.events,
        )
    })
    .expect("diario");
    assert_eq!(summary.csv, Csv::Reused, "the other machine's CSV is used");
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        0,
        "no download while the lease is held"
    );
    assert_eq!(summary.run.finished.done, 2);
}

#[test]
fn losing_the_download_race_also_ends_up_reusing_the_winners_csv() {
    let env = setup();
    let (id, _) = cargue_of(day());
    let lease = format!("convenio_descargar:{id}");
    let attempts = AtomicUsize::new(0);
    let download = |ctx: &AppContext, _: NaiveDate| {
        attempts.fetch_add(1, Ordering::SeqCst);
        // The rival grabbed the lease between our check and our download.
        ctx.leases()
            .acquire(&lease, "rival", Duration::from_secs(60))
            .expect("lease");
        Err(anyhow!("otra máquina ya está descargando el cargue {id}"))
    };
    let (log, rad) = (Log::default(), batches());
    let out = capture(Mode::Json);
    let summary = std::thread::scope(|scope| {
        let (env, lease) = (&env, &lease);
        scope.spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            register(env, &env.ctx.clone().with_owner("rival"), 2);
            env.ctx.leases().release(lease, "rival").expect("release");
        });
        run_day(
            &env.ctx,
            day(),
            &config("ignored"),
            false,
            &download,
            &fake_runner(&log, &rad),
            &out.events,
        )
    })
    .expect("diario");
    assert_eq!(
        (summary.csv, attempts.load(Ordering::SeqCst)),
        (Csv::Reused, 1)
    );
    assert_eq!(summary.run.finished.done, 2);
}

#[test]
fn a_failed_download_without_a_competing_machine_is_an_error() {
    let env = setup();
    let download = |_: &AppContext, _: NaiveDate| Err(anyhow!("medicar: 500"));
    let (log, rad) = (Log::default(), batches());
    let out = capture(Mode::Json);
    let err = run_day(
        &env.ctx,
        day(),
        &config("ignored"),
        false,
        &download,
        &fake_runner(&log, &rad),
        &out.events,
    )
    .expect_err("download failed");
    let text = format!("{err:#}");
    assert!(
        text.contains("descarga del convenio") && text.contains("medicar: 500"),
        "{text}"
    );
    assert!(log.all().is_empty() && out.kinds().is_empty());
}

#[test]
fn a_failed_invoice_makes_the_day_fail_and_the_summary_is_in_spanish() {
    let env = setup();
    let download = |ctx: &AppContext, _: NaiveDate| Ok(register(&env, ctx, 3));
    let (log, rad) = (Log::default(), batches());
    let runner = fake_runner_with(&log, &rad, |phase| -> Decide {
        if phase == "cuv" {
            Arc::new(|invoice| {
                if invoice == "EVS2" {
                    Outcome::failed("SISPRO caído")
                } else {
                    Outcome::done()
                }
            })
        } else {
            always_done()
        }
    });
    let out = capture(Mode::Json);
    let summary = run_day(
        &env.ctx,
        day(),
        &config("ignored"),
        false,
        &download,
        &runner,
        &out.events,
    )
    .expect("run");
    assert!(summary.failed());
    let lines = summary.lines();
    assert_eq!(
        lines[0],
        format!("diario 2026-10-21: cargue {ID} (CSV descargado, preparado ahora)")
    );
    assert_eq!(
        lines[1],
        "facturas: 3 procesadas, 2 terminadas, 0 en revisión manual, 0 excluidas, 1 fallidas, 0 omitidas (otra máquina)"
    );
}

#[test]
fn two_diario_runs_of_the_same_day_at_once_do_not_duplicate_anything() {
    let env = setup();
    register(&env, &env.ctx, 12);
    let log = Log::default();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let downloads = Arc::new(AtomicUsize::new(0));
    let rads: Vec<_> = (0..2).map(|_| batches()).collect();
    std::thread::scope(|scope| {
        for (n, rad) in rads.iter().enumerate() {
            let ctx = env.ctx.clone().with_owner(&format!("cron-{n}"));
            let (log, barrier, downloads, rad) =
                (log.clone(), barrier.clone(), downloads.clone(), rad.clone());
            scope.spawn(move || {
                let download = |_: &AppContext, _: NaiveDate| {
                    downloads.fetch_add(1, Ordering::SeqCst);
                    Err(anyhow!("already registered, never downloaded"))
                };
                let out = capture(Mode::Json);
                barrier.wait();
                let summary = run_day(
                    &ctx,
                    day(),
                    &config("ignored"),
                    false,
                    &download,
                    &fake_runner(&log, &rad),
                    &out.events,
                )
                .expect("diario");
                assert!(!summary.failed());
            });
        }
    });
    assert_eq!(downloads.load(Ordering::SeqCst), 0);
    for n in 1..=12 {
        for phase in ["thefactory", "fev", "derechos", "rips", "cuv", "soporte"] {
            assert_eq!(log.count(phase, &format!("EVS{n}")), 1, "{phase} EVS{n}");
        }
    }
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
    assert_eq!(invoices.len(), 12);
    assert!(
        invoices.iter().all(|i| i.version == 0),
        "prepared exactly once"
    );
}
