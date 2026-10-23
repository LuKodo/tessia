//! Fixture of the `api_read_a` tests: a synthetic equivalent of the Go golden
//! fixture of the dashboard (`cmd/tessia/cmd_serve_golden_test.go`), seeded
//! through the Rust stores and raw SQL with explicit update times.
//!
//! One deliberate difference: Rust stores invoice names normalized, so the Go
//! invoice `fvs-3` is `FVS3` here (and sorts between FVS2 and FVS4).
#![allow(dead_code)]

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use diesel::RunQueryDsl;
use serde_json::Value;
use tessia::api::{ApiState, Request};
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::dataset::test_support::{DatasetBuilder, InvoiceBuilder, sample_line};
use tessia::domain::dataset::{Dataset, Line};
use tessia::jobs::JobRunner;
use tessia::stores::cargues::{CarguesStore, NewCargue};
use tessia::stores::dataset::DatasetStore;
use tessia::stores::dataset::test_support::seed_cargue;

pub struct Fixture {
    pub db: TestDb,
    pub state: ApiState,
}

pub fn state_for(db: &TestDb) -> ApiState {
    let ctx = AppContext::for_tests(db);
    ApiState::new(ctx.clone(), db.pool(), Arc::new(JobRunner::new(ctx)))
}

pub fn empty_fixture() -> Fixture {
    let db = TestDb::new();
    let state = state_for(&db);
    Fixture { db, state }
}

pub fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("date")
}

pub fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("time")
        .with_timezone(&Utc)
}

/// Runs a statement (test data only; every value is a literal of the fixture).
pub fn exec(db: &TestDb, sql: &str) {
    diesel::sql_query(sql).execute(&mut db.conn()).expect(sql);
}

pub fn proceso(db: &TestDb, invoice: &str, phase: &str, state: &str, reason: &str, at: &str) {
    exec(
        db,
        &format!(
            "INSERT INTO proceso_estado (num_factura, fase, estado, motivo, intentos, created_at, updated_at)
             VALUES ('{invoice}', '{phase}', '{state}', '{reason}', 1, '{at}', '{at}')"
        ),
    );
}

pub fn radicacion(db: &TestDb, invoice: &str, step: i32, status: &str, detail: &str, at: &str) {
    exec(
        db,
        &format!(
            "INSERT INTO radicacion_estado (invoice, step, status, last_detail, created_at, updated_at)
             VALUES ('{invoice}', {step}, '{status}', '{detail}', '{at}', '{at}')"
        ),
    );
}

pub fn portal(db: &TestDb, invoice: &str, estado: &str, warnings: i32, errors: i32, fin: bool) {
    exec(
        db,
        &format!(
            "INSERT INTO portal_cargue (num_factura, estado, total_advertencias, total_errores, es_final)
             VALUES ('{invoice}', '{estado}', {warnings}, {errors}, {fin})"
        ),
    );
}

/// The two lines of every golden invoice.
pub fn lines(invoice: &str, subplan: &str, ssc: &str) -> Vec<Line> {
    let mut first = sample_line(invoice, 1, "100.50");
    first.subplan = subplan.into();
    first.ssc = ssc.into();
    first.date = "2026-09-03 11:28:31".into();
    first.authorization = format!("AUT-{invoice}");
    first.mipres = "M1".into();
    first.cum = "CUM1".into();
    first.agreed_price = "40.25".into();
    first.unit_value = "50".into();
    let mut second = sample_line(invoice, 2, "9.5");
    second.subplan = subplan.into();
    second.ssc = ssc.into();
    second.date = "2026-09-03 11:28:31".into();
    second.authorization = String::new();
    second.mipres = String::new();
    second.cum = String::new();
    second.agreement_cum = "CUM2".into();
    second.unit_value = "7".into();
    vec![first, second]
}

fn invoice(name: &str, subplan: &str, ssc: &str) -> InvoiceBuilder {
    lines(name, subplan, ssc)
        .into_iter()
        .fold(InvoiceBuilder::new(name), |b, l| b.with_line(l))
}

fn with_header(mut ds: Dataset, from: &str, to: &str, at: &str) -> Dataset {
    ds.preparation.load.from = Some(day(from));
    ds.preparation.load.to = Some(day(to));
    ds.preparation.prepared_at = Some(utc(at));
    ds
}

/// Seeds one prepared load.
pub fn store_load(
    db: &TestDb,
    file: &str,
    hash: char,
    build: impl FnOnce(DatasetBuilder) -> DatasetBuilder,
    from: &str,
    to: &str,
    at: &str,
) -> i64 {
    let seq = seed_cargue(&db.pool(), file, hash);
    let ds = with_header(
        build(DatasetBuilder::new(seq).load(file)).build(),
        from,
        to,
        at,
    );
    DatasetStore::new(db.pool())
        .replace(&ds, false)
        .expect("replace");
    seq
}

pub fn golden_fixture() -> Fixture {
    let db = TestDb::new();
    store_load(
        &db,
        "CargueA.csv",
        'a',
        |b| {
            b.invoice(invoice("FVS1", "EVENTO NO PBS", "SSC1").mipres("111, 222"))
                .invoice(invoice("FVS2", "EVENTO POS", "SSC2"))
                .invoice(invoice("FVS3", "EVENTO POS", "SSC3"))
                .invoice(invoice("FVS4", "EVENTO POS", "SSC4"))
                .invoice(invoice("FVS5", "EVENTO POS", "SSC5"))
                .invoice(invoice("FVS6", "EVENTO POS", "SSC6"))
                .invoice(invoice("FVS7", "EVENTO POS", ""))
                .excluded("FVS8", "MONTO_MAXIMO_SUPERADO")
                .manual_review("FVS9", "SIN_SSC", "sin ssc")
        },
        "2026-09-01",
        "2026-09-25",
        "2026-09-20T08:00:00Z",
    );
    store_load(
        &db,
        "CargueB.csv",
        'b',
        |b| {
            b.invoice(invoice("FVS20", "EVENTO POS", "SSC20"))
                .invoice(invoice("FVS21", "EVENTO POS", "SSC21"))
        },
        "2026-09-26",
        "2026-09-30",
        "2026-09-28T08:00:00Z",
    );
    // A cargue that was uploaded but never prepared.
    let info = tessia::domain::cargue::load_context("Convenio_01092026_02092026.csv");
    let mut new = NewCargue::from_load(&info, &"c".repeat(64));
    new.id = "CargueCSV".into();
    CarguesStore::new(db.pool()).ingest(&new).expect("ingest");
    exec(
        &db,
        "UPDATE cargues SET created_at = '2026-09-05T08:00:00Z' WHERE id = 'CargueCSV'",
    );
    seed_native(&db);
    let state = state_for(&db);
    Fixture { db, state }
}

fn seed_native(db: &TestDb) {
    for (inv, phase, st, reason, at) in [
        ("FVS1", "derechos", "DONE", "", "2026-09-10 09:10:00"),
        ("FVS1", "thefactory", "DONE", "", "2026-09-10 09:11:00"),
        ("FVS1", "fev_ajuste", "DONE", "", "2026-09-10 09:12:00"),
        ("FVS1", "fev_valor", "DONE", "", "2026-09-10 09:13:00"),
        ("FVS1", "rips", "DONE", "", "2026-09-10 09:14:00"),
        ("FVS1", "cuv", "DONE", "", "2026-09-10 09:15:00"),
        ("FVS1", "soporte", "DONE", "", "2026-09-10 09:16:00"),
        ("FVS2", "derechos", "DONE", "", "2026-09-11 09:10:00"),
        ("FVS2", "thefactory", "DONE", "", "2026-09-11 09:11:00"),
        (
            "FVS2",
            "rips",
            "FAILED",
            "DESCARGA: timeout",
            "2026-09-11 09:12:00",
        ),
        ("FVS3", "derechos", "DONE", "", "2026-09-12 07:10:00"),
        (
            "FVS3",
            "thefactory",
            "REVISION_MANUAL",
            "SIN_XML: falta",
            "2026-09-12 07:11:00",
        ),
        ("FVS3", "fev_valor", "DONE", "", "2026-09-12 07:12:00"),
        (
            "FVS3",
            "conciliacion",
            "EXCLUDED",
            "BLOQUEADA_TARIFA: precio del XML supera la nota tecnica",
            "2026-09-12 07:20:00",
        ),
        ("FVS4", "derechos", "EN_PROGRESO", "", "2026-09-13 08:10:00"),
        ("FVS5", "derechos", "DONE", "", "2026-09-14 08:10:00"),
        ("FVS5", "thefactory", "DONE", "", "2026-09-14 08:11:00"),
        ("FVS5", "fev_ajuste", "DONE", "", "2026-09-14 08:12:00"),
        (
            "FVS6",
            "derechos",
            "REVISION_MANUAL",
            "SIN_DERECHOS: no vigente",
            "2026-09-14 08:13:00",
        ),
        ("FVS20", "soporte", "DONE", "", "2026-09-28 07:30:00"),
        // Rows outside every cargue.
        ("FVSX1", "derechos", "DONE", "x1", "2026-09-29 07:00:00"),
        ("FVSX2", "derechos", "DONE", "", "2026-09-29 07:00:00"),
    ] {
        proceso(db, inv, phase, st, reason, at);
    }
    radicacion(
        db,
        "FVS1",
        4,
        "CONFIRMED",
        "Radicada completa",
        "2026-09-10 10:00:00",
    );
    radicacion(
        db,
        "FVS2",
        2,
        "SENT",
        "esperando respuesta",
        "2026-09-11 09:30:00",
    );
    radicacion(
        db,
        "FVS3",
        1,
        "BLOCKED",
        "Paso 1 bloqueado: VALOR_FACTURA_DIFERENTE: el total del XML no coincide",
        "2026-09-12 09:30:00",
    );
    radicacion(
        db,
        "FVS4",
        3,
        "ADJUSTMENT_NOTE",
        "requiere nota ajuste",
        "2026-09-13 08:00:00",
    );
    radicacion(
        db,
        "FVS5",
        1,
        "CREDIT_NOTE",
        "RFFE022: superó los 22 días hábiles",
        "2026-09-14 08:00:00",
    );
    radicacion(
        db,
        "FVS6",
        4,
        "CREDIT_NOTE",
        "requiere nota credito",
        "2026-09-14 08:30:00",
    );
    radicacion(db, "FVS20", 4, "CONFIRMED", "", "2026-09-28 08:00:00");
    radicacion(db, "FVSX2", 2, "PENDING", "x2", "2026-09-29 07:00:00");
    portal(db, "FVS1", "Aprobado", 1, 0, true);
    portal(db, "FVS3", "Con errores", 0, 2, false);
}

/// Sends `GET path?query` and returns the status and the parsed JSON body.
pub fn get(state: &ApiState, path: &str, query: &[(&str, &str)]) -> (u16, Value) {
    let mut req = Request::new("GET", path);
    for (k, v) in query {
        req = req.with_query(k, v);
    }
    let res = state.handle(req);
    assert_eq!(
        res.header("Content-Type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(res.body.last(), Some(&b'\n'), "trailing newline");
    (
        res.status,
        serde_json::from_slice(&res.body).expect("json body"),
    )
}

pub fn raw(state: &ApiState, path: &str, query: &[(&str, &str)]) -> String {
    let mut req = Request::new("GET", path);
    for (k, v) in query {
        req = req.with_query(k, v);
    }
    String::from_utf8(state.handle(req).body).expect("utf8")
}
