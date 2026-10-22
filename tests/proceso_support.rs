//! Shared helpers of the `proceso_*` and `diario_*` integration tests
//! (included with `#[path = "proceso_support.rs"] mod support;`).
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::{Outcome, Phase, PhaseState};
use tessia::nt::{MemoryNt, NtRow};
use tessia::phases::common::{PhaseReport, PhaseRequest};
use tessia::phases::proceso::{Event, Events, Mode, PhaseRunner, RunConfig};
use tessia::stores::proceso::ClaimItem;

pub const ID: &str = "Convenio_21102026_21102026";
const HEADER: &str = "nit|plan|subplan|factura|tipo de identificacion|no. documento|no. formula|fecha|plu|articulo|cantidad|vlr. unitario|vlr. total|cuota|mipres|cum";
pub const OTHER_MACHINE: &str = "EN_PROCESO_OTRA_MAQUINA";

fn nt_row(code: &str, cum: &str, product_type: &str, price: &str) -> NtRow {
    NtRow {
        cod_gen_siesa: Some(code.into()),
        cum_facturar: Some(cum.into()),
        tipo_producto: Some(product_type.into()),
        precio_contrato: Some(price.parse().expect("price")),
        ffm_rips: Some("TABLETA".into()),
        umm_rips: Some("500".into()),
        upr_rips: Some("MG".into()),
        ..NtRow::default()
    }
}

pub fn catalog() -> MemoryNt {
    MemoryNt::new().with_version(
        "2026-10",
        vec![
            ("1".into(), nt_row("20010", "19950404-1", "714", "1500")),
            ("2".into(), nt_row("30020", "12345", "799", "0")),
        ],
    )
}

/// A POS invoice that prepares fine.
pub fn pos(invoice: &str, document: &str) -> String {
    format!(
        "900000001|PLAN|EVENTO POS|{invoice}|CC|{document}||2026-10-21 10:00:00|20010_01|PRODUCTO|1|1500|1500|0||19950404-1"
    )
}

/// An invoice the preparation excludes (its PLU is not in the nota técnica).
pub fn excluded(invoice: &str, document: &str) -> String {
    format!(
        "900000001|PLAN|EVENTO POS|{invoice}|CC|{document}||2026-10-21 10:00:00|99999_01|PRODUCTO|1|1|1|0||000"
    )
}

pub fn csv(rows: &[String]) -> String {
    format!("{HEADER}\n{}\n", rows.join("\n"))
}

/// `n` prepared invoices `EVS1..EVSn`.
pub fn pos_rows(n: usize) -> Vec<String> {
    (1..=n)
        .map(|i| pos(&format!("EVS-{i}"), &format!("{}", 1000 + i)))
        .collect()
}

pub fn write_csv(dir: &Path, name: &str, rows: &[String]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, csv(rows)).expect("write csv");
    path
}

pub struct Env {
    pub _db: TestDb,
    pub ctx: AppContext,
    pub dir: tempfile::TempDir,
}

pub fn setup() -> Env {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db).with_nt(Arc::new(catalog()));
    Env {
        _db: db,
        ctx,
        dir: tempfile::tempdir().expect("tempdir"),
    }
}

/// A config with the knobs shortened for tests.
pub fn config(source: &str) -> RunConfig {
    let mut cfg = RunConfig::new(source);
    cfg.max_amount = Some("250000".into());
    cfg.radicacion_interval = Duration::ZERO;
    cfg.tuning.radicacion_retry = Duration::from_millis(20);
    cfg.tuning.radicacion_wait = Duration::from_millis(400);
    cfg.tuning.prepare_poll = Duration::from_millis(20);
    cfg.tuning.prepare_wait = Duration::from_secs(10);
    for pool in [
        "thefactory",
        "fev",
        "conciliar",
        "derechos",
        "rips",
        "cuv",
        "soporte",
    ] {
        cfg.workers.insert(pool, 4);
    }
    cfg
}

#[derive(Clone, Default)]
pub struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("buffer").extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Buffer {
    pub fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("buffer").clone()).expect("utf8")
    }
}

/// An event sink that records every event and the rendered output.
pub struct Capture {
    pub events: Events,
    pub seen: Arc<Mutex<Vec<Event>>>,
    pub out: Buffer,
}

pub fn capture(mode: Mode) -> Capture {
    let out = Buffer::default();
    let seen: Arc<Mutex<Vec<Event>>> = Arc::default();
    let tap = seen.clone();
    let events = Events::new(mode, Box::new(out.clone()), false)
        .with_tap(move |e| tap.lock().expect("events").push(e.clone()));
    Capture { events, seen, out }
}

impl Capture {
    pub fn of(&self, kind: &str) -> Vec<Event> {
        self.seen
            .lock()
            .expect("events")
            .iter()
            .filter(|e| e.kind == kind)
            .cloned()
            .collect()
    }

    pub fn kinds(&self) -> Vec<String> {
        self.seen
            .lock()
            .expect("events")
            .iter()
            .map(|e| e.kind.clone())
            .collect()
    }
}

/// One unit of work a fake phase did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Work {
    pub phase: String,
    pub invoice: String,
    pub owner: String,
}

#[derive(Clone, Default)]
pub struct Log(Arc<Mutex<Vec<Work>>>);

impl Log {
    pub fn all(&self) -> Vec<Work> {
        self.0.lock().expect("log").clone()
    }

    pub fn push(&self, phase: &str, invoice: &str, owner: &str) {
        self.0.lock().expect("log").push(Work {
            phase: phase.to_owned(),
            invoice: invoice.to_owned(),
            owner: owner.to_owned(),
        });
    }

    /// The phases an invoice went through, in order.
    pub fn path(&self, invoice: &str) -> Vec<String> {
        self.all()
            .into_iter()
            .filter(|w| w.invoice == invoice)
            .map(|w| w.phase)
            .collect()
    }

    pub fn count(&self, phase: &str, invoice: &str) -> usize {
        self.all()
            .iter()
            .filter(|w| w.phase == phase && w.invoice == invoice)
            .count()
    }

    pub fn phase_calls(&self, phase: &str) -> usize {
        self.all().iter().filter(|w| w.phase == phase).count()
    }
}

/// What a fake phase decides for an invoice.
pub type Decide = Arc<dyn Fn(&str) -> Outcome + Send + Sync>;

pub fn always_done() -> Decide {
    Arc::new(|_| Outcome::done())
}

/// A fake phase that behaves like the real ones with respect to the
/// multi-machine contract: it claims every invoice in `proceso_estado`,
/// reports `EN_PROCESO_OTRA_MAQUINA` for what another machine owns, answers
/// `DONE` without work for what is already done (like `YA_EXISTE`), and
/// stores its outcome.
pub fn claiming_phase(
    phase: Phase,
    name: &'static str,
    log: Log,
    decide: Decide,
    pause: Duration,
) -> impl Fn(&AppContext, &PhaseRequest) -> Result<PhaseReport> + Send + Sync + 'static {
    move |ctx, req| {
        let invoices = req.invoices.clone().unwrap_or_default();
        let store = ctx.proceso();
        let loaded = store.load(phase, Some(&invoices))?;
        let mut report = PhaseReport::default();
        let mut items = Vec::new();
        for invoice in &invoices {
            if loaded
                .get(invoice)
                .is_some_and(|s| s.state == PhaseState::Done)
            {
                report.push(invoice, Outcome::done());
            } else {
                items.push(ClaimItem::from_loaded(invoice, loaded.get(invoice)));
            }
        }
        let claimed = store.claim_available(phase, &items, &ctx.claim_options(&req.cargue_id))?;
        for item in &items {
            let Some(version) = claimed.get(&item.invoice) else {
                report.skip(&item.invoice, OTHER_MACHINE);
                continue;
            };
            log.push(name, &item.invoice, &ctx.owner);
            std::thread::sleep(pause);
            let outcome = decide(&item.invoice);
            if store.save_outcome(phase, &item.invoice, &outcome, *version)? {
                report.push(&item.invoice, outcome);
            } else {
                report.skip(&item.invoice, OTHER_MACHINE);
            }
        }
        Ok(report)
    }
}

/// Fake `radicacion` recording its batches (and a unit of work per invoice).
pub fn recording_radicacion(
    batches: Arc<Mutex<Vec<Vec<String>>>>,
) -> impl Fn(&AppContext, &PhaseRequest) -> Result<PhaseReport> + Send + Sync + 'static {
    move |_, req| {
        let invoices = req.invoices.clone().unwrap_or_default();
        batches.lock().expect("batches").push(invoices.clone());
        let mut report = PhaseReport::default();
        for invoice in &invoices {
            report.push(invoice, Outcome::done());
        }
        Ok(report)
    }
}

/// A runner whose mainline phases are claiming fakes that finish `DONE`.
pub fn fake_runner(log: &Log, batches: &Arc<Mutex<Vec<Vec<String>>>>) -> PhaseRunner {
    fake_runner_with(log, batches, |_| always_done())
}

/// Like [`fake_runner`] with a per-phase decision.
pub fn fake_runner_with(
    log: &Log,
    batches: &Arc<Mutex<Vec<Vec<String>>>>,
    decide: impl Fn(&'static str) -> Decide,
) -> PhaseRunner {
    let phases = [
        (Phase::TheFactory, "thefactory"),
        (Phase::FevValor, "fev"),
        (Phase::Conciliacion, "conciliar"),
        (Phase::Derechos, "derechos"),
        (Phase::Rips, "rips"),
        (Phase::Cuv, "cuv"),
        (Phase::Soporte, "soporte"),
    ];
    let mut runner = PhaseRunner::new();
    for (phase, name) in phases {
        runner = runner.with(
            name,
            claiming_phase(
                phase,
                name,
                log.clone(),
                decide(name),
                Duration::from_millis(1),
            ),
        );
    }
    runner.with("radicacion", recording_radicacion(batches.clone()))
}

pub fn batches() -> Arc<Mutex<Vec<Vec<String>>>> {
    Arc::default()
}

pub fn flatten(batches: &Arc<Mutex<Vec<Vec<String>>>>) -> Vec<String> {
    let mut all: Vec<String> = batches
        .lock()
        .expect("batches")
        .iter()
        .flatten()
        .cloned()
        .collect();
    all.sort();
    all
}
