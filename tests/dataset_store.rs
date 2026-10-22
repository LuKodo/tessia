//! `DatasetStore` against a real Postgres (`TestDb`), through the public API.

#![allow(clippy::field_reassign_with_default)]

use std::sync::{Arc, Barrier};

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::LoadOrigin;
use tessia::domain::dataset::test_support::{DatasetBuilder, InvoiceBuilder, sample_line};
use tessia::domain::dataset::{
    Dataset, Invoice, InvoiceAudit, InvoiceState, Line, LineAudit, Stats,
};
use tessia::stores::cargues::{CarguesStore, NewCargue};
use tessia::stores::dataset::test_support::seed_cargue;
use tessia::stores::dataset::{DatasetError, DatasetStore};

const LOAD: &str = "Convenio_21102026_21102026";

struct Fixture {
    db: TestDb,
    store: DatasetStore,
    cargue: i64,
}

fn fixture() -> Fixture {
    let db = TestDb::new();
    let store = DatasetStore::new(db.pool());
    let cargue = seed_cargue(&db.pool(), LOAD, 'a');
    Fixture { db, store, cargue }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn dec(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

/// A line with every typed field set to a distinct value.
fn full_line() -> Line {
    let mut l = Line::default();
    l.source_line = 7;
    l.nit = "nit".into();
    l.plan = "plan".into();
    l.subplan = "EVENTO NO PBS".into();
    l.invoice = "EVS1".into();
    l.invoice_raw = "evs-1".into();
    l.doc_type = "cc".into();
    l.doc_number = "123".into();
    l.beneficiary = "beneficiario ñandú".into();
    l.point_of_sale = "pos".into();
    l.formula_number = "f1".into();
    l.ssc = "ssc1".into();
    l.formula_type = "tf".into();
    l.date = "21/10/2026".into();
    l.lot = "lote".into();
    l.plu = "plu".into();
    l.article = "artículo largo".into();
    l.quantity = "2".into();
    l.unit_value = "1.234,50".into();
    l.unit_cost = "1000".into();
    l.total_value = "2.469,00".into();
    l.copay = "100".into();
    l.authorization = "aut".into();
    l.mipres = "mp".into();
    l.agreement_cum = "1-1".into();
    l.diagnosis_code = "A00".into();
    l.diagnosis_name = "diag".into();
    l.causal_exento = "ESS207".into();
    l.ips = "ips".into();
    l.prescriber_id = "pid".into();
    l.prescriber_name = "pname".into();
    l.gender = "F".into();
    l.birth_date = "2000-01-01".into();
    l.department = "05001".into();
    l.municipality = "U".into();
    l.classification = "714".into();
    l.cum = "2-2".into();
    l.product_type = "714".into();
    l.pharmaceutical_form = "TABLETA".into();
    l.concentration = "500 mg".into();
    l.measure_unit = "mg".into();
    l.agreed_price = "1.200,00".into();
    l.conciliacion = Some(LineAudit {
        origin: "CSV_CONCILIADO_XML".into(),
        xml_identifier: "3".into(),
        date_source: "MEDICAR".into(),
        value_source: "XML".into(),
    });
    l.extra.insert("futuro".into(), serde_json::json!([1, "a"]));
    l
}

#[test]
fn replace_then_read_round_trips_every_typed_field() {
    let f = fixture();
    let at = Utc.with_ymd_and_hms(2026, 10, 21, 8, 30, 15).unwrap();
    let audit = InvoiceAudit {
        state: "CONCILIADA".into(),
        formula_date: "2026-10-20".into(),
        ssc: "ssc1".into(),
        xml_total: "10".into(),
        previous_total: "9".into(),
        reconciled_total: "10".into(),
        reconciled_at: Some(at),
        original_lines: vec![sample_line("EVS1", 1, "9")],
        unconfirmed: vec![0, 2],
    };
    let mut ds = DatasetBuilder::new(f.cargue)
        .load(LOAD)
        .nt_version("2026-10-21T08:00")
        .max_amount(dec("1500000.50"))
        .prepared_format(true)
        .prepared_at(at)
        .stats(Stats {
            medicines: 1,
            other_services: 2,
            copay_lines: 3,
            dropped_no_invoice: 4,
            ignored: 5,
            finished: 6,
            evaluated: 7,
            prepared_pending: 8,
            limit: 9,
            mipres_unavailable: 10,
        })
        .invoice(
            InvoiceBuilder::new("EVS1")
                .with_line(full_line())
                .line("5")
                .mipres("11, 22")
                .audit(audit),
        )
        .excluded("EVS2", "VALOR_SUPERA_MONTO")
        .manual_review("EVS3", "MULTIPLES_AFILIADOS_EN_FACTURA", "dos afiliados")
        .build();
    ds.preparation.skipped_lines = vec![4, 9];
    ds.preparation.source = "origen.csv".into();
    ds.preparation.load.origin = LoadOrigin::Medicar;
    f.store.replace(&ds, false).expect("replace");

    let read = f.store.read(f.cargue, None).expect("read");
    assert_eq!(read.cargue, f.cargue);
    assert_eq!(read.preparation, ds.preparation);
    assert_eq!(read.excluded, ds.excluded);
    assert_eq!(read.manual_review, ds.manual_review);
    assert_eq!(read.invoices.len(), 1);
    let inv = &read.invoices[0];
    assert_eq!(inv.version, 0);
    assert_eq!(inv.lines, ds.invoices[0].lines);
    assert_eq!(inv.lines[0], full_line_with_forced_invoice());
    assert_eq!(inv.id_mipres, "11, 22");
    assert_eq!(inv.conciliacion, ds.invoices[0].conciliacion);
    assert_eq!(inv.agreement_total().unwrap(), dec("2474.00"));
}

fn full_line_with_forced_invoice() -> Line {
    let mut l = full_line();
    l.invoice = "EVS1".into();
    l
}

#[test]
fn unset_prepared_at_uses_the_database_clock_and_missing_dataset_is_not_found() {
    let f = fixture();
    assert!(matches!(
        f.store.read(f.cargue, None),
        Err(DatasetError::NotFound(c)) if c == f.cargue
    ));
    assert_eq!(f.store.preparation(f.cargue).unwrap(), None);
    let before = Utc::now() - chrono::Duration::minutes(5);
    f.store
        .replace(&DatasetBuilder::new(f.cargue).build(), false)
        .expect("replace");
    let at = f
        .store
        .preparation(f.cargue)
        .unwrap()
        .and_then(|p| p.prepared_at)
        .expect("prepared_at");
    assert!(at > before && at < Utc::now() + chrono::Duration::minutes(5));
}

#[test]
fn replace_for_an_unknown_cargue_is_cargue_not_found() {
    let f = fixture();
    let ds = DatasetBuilder::new(f.cargue + 999).build();
    assert!(matches!(
        f.store.replace(&ds, false),
        Err(DatasetError::CargueNotFound(_))
    ));
}

fn three_invoices(cargue: i64) -> Dataset {
    DatasetBuilder::new(cargue)
        .invoice(InvoiceBuilder::new("EVS1").line("10"))
        .invoice(InvoiceBuilder::new("EVS2").line("20").line("1"))
        .excluded("EVS3", "X")
        .manual_review("EVS4", "C", "R")
        .invoice(InvoiceBuilder::new("EVS5").line("50"))
        .build()
}

#[test]
fn subset_reads_return_only_the_named_invoices_with_the_whole_header() {
    let f = fixture();
    let mut ds = three_invoices(f.cargue);
    ds.preparation.nt_version = "v9".into();
    f.store.replace(&ds, false).expect("replace");

    let sub = f
        .store
        .read(
            f.cargue,
            Some(&names(&["evs-2", "EVS2", "EVS3", "EVS4", "NOPE", " "])),
        )
        .expect("subset");
    assert_eq!(
        sub.preparation,
        f.store.read(f.cargue, None).unwrap().preparation
    );
    assert_eq!(sub.preparation.nt_version, "v9");
    let got: Vec<&str> = sub.invoices.iter().map(|i| i.invoice.as_str()).collect();
    assert_eq!(got, ["EVS2"]);
    assert_eq!(sub.invoices[0].lines.len(), 2);
    assert_eq!(sub.excluded.len(), 1);
    assert_eq!(sub.manual_review.len(), 1);
    // Same result as reading everything and filtering in memory.
    let full = f.store.read(f.cargue, None).unwrap();
    assert_eq!(
        sub,
        full.filtered(&names(&["EVS2", "EVS3", "EVS4", "NOPE"]))
    );

    let header_only = f.store.read(f.cargue, Some(&[])).expect("header only");
    assert!(header_only.all_names().is_empty());
    assert_eq!(header_only.preparation, full.preparation);

    let order = f
        .store
        .read_invoices(f.cargue, Some(&names(&["EVS5", "EVS1"])))
        .unwrap();
    let order: Vec<&str> = order.iter().map(|i| i.invoice.as_str()).collect();
    assert_eq!(order, ["EVS1", "EVS5"], "stored order, not request order");
}

#[test]
fn names_by_state_follow_the_stored_order() {
    let f = fixture();
    f.store.replace(&three_invoices(f.cargue), false).unwrap();
    assert_eq!(
        f.store.invoice_names(f.cargue).unwrap(),
        ["EVS1", "EVS2", "EVS5"]
    );
    assert_eq!(
        f.store
            .names_with_state(f.cargue, InvoiceState::Excluded)
            .unwrap(),
        ["EVS3"]
    );
    assert_eq!(
        f.store
            .names_with_state(f.cargue, InvoiceState::ManualReview)
            .unwrap(),
        ["EVS4"]
    );
}

#[test]
fn storing_an_invoice_again_bumps_its_version() {
    let f = fixture();
    f.store.replace(&three_invoices(f.cargue), false).unwrap();
    f.store.replace(&three_invoices(f.cargue), false).unwrap();
    let read = f.store.read(f.cargue, None).unwrap();
    assert!(read.invoices.iter().all(|i| i.version == 1));
    // A new invoice in a later replace starts at 0.
    let mut ds = three_invoices(f.cargue);
    ds.invoices
        .push(InvoiceBuilder::new("EVS6").line("6").build());
    f.store.replace(&ds, false).unwrap();
    let read = f.store.read(f.cargue, None).unwrap();
    let versions: Vec<(&str, i64)> = read
        .invoices
        .iter()
        .map(|i| (i.invoice.as_str(), i.version))
        .collect();
    assert_eq!(
        versions,
        [("EVS1", 2), ("EVS2", 2), ("EVS5", 2), ("EVS6", 0)]
    );
}

#[test]
fn merge_replace_keeps_untouched_invoices_and_appends_the_new_ones() {
    let f = fixture();
    let mut first = three_invoices(f.cargue);
    first.preparation.stats.medicines = 41;
    first.preparation.skipped_lines = vec![3];
    first.preparation.prepared_format = true;
    f.store.replace(&first, false).unwrap();

    // Re-prepare EVS2 (new lines) and move EVS5 to excluded.
    let mut second = DatasetBuilder::new(f.cargue)
        .load(LOAD)
        .nt_version("v2")
        .invoice(InvoiceBuilder::new("EVS2").line("99"))
        .excluded("EVS5", "AHORA_EXCLUIDA")
        .build();
    second.preparation.stats.medicines = 1;
    second.preparation.prepared_format = false;
    f.store.replace(&second, true).unwrap();

    let read = f.store.read(f.cargue, None).unwrap();
    let prepared: Vec<(&str, i64)> = read
        .invoices
        .iter()
        .map(|i| (i.invoice.as_str(), i.version))
        .collect();
    assert_eq!(prepared, [("EVS1", 0), ("EVS2", 1)]);
    assert_eq!(read.invoices[1].lines[0].total_value, "99");
    let excluded: Vec<&str> = read.excluded.iter().map(|e| e.invoice.as_str()).collect();
    assert_eq!(excluded, ["EVS3", "EVS5"]);
    assert_eq!(read.manual_review.len(), 1);
    // Kept stats win, prepared_format is the OR, the rest of the header is new.
    assert_eq!(read.preparation.stats.medicines, 41);
    assert_eq!(read.preparation.skipped_lines, vec![3]);
    assert!(read.preparation.prepared_format);
    assert_eq!(read.preparation.nt_version, "v2");
    // The merged invoice moved to the end, after the kept ones.
    assert_eq!(f.store.invoice_names(f.cargue).unwrap(), ["EVS1", "EVS2"]);
}

#[test]
fn replace_touching_drops_requested_names_the_dataset_does_not_contain() {
    let f = fixture();
    f.store.replace(&three_invoices(f.cargue), false).unwrap();
    let only = DatasetBuilder::new(f.cargue)
        .load(LOAD)
        .invoice(InvoiceBuilder::new("EVS1").line("1"))
        .build();
    f.store
        .replace_touching(&only, &names(&["EVS1", "evs-2"]))
        .unwrap();
    let read = f.store.read(f.cargue, None).unwrap();
    let all = read.all_names();
    assert_eq!(all, ["EVS5", "EVS1", "EVS3", "EVS4"].map(String::from));
}

#[test]
fn merge_into_another_load_or_an_empty_cargue_replaces_everything() {
    let f = fixture();
    f.store.replace(&three_invoices(f.cargue), false).unwrap();
    let other_load = DatasetBuilder::new(f.cargue)
        .load("Manual_1_abc")
        .invoice(InvoiceBuilder::new("EVS9").line("9"))
        .build();
    f.store.replace(&other_load, true).unwrap();
    assert_eq!(f.store.read(f.cargue, None).unwrap().all_names(), ["EVS9"]);

    let fresh = seed_cargue(&f.db.pool(), "Manual_2_def", 'b');
    let merge = DatasetBuilder::new(fresh)
        .load("Manual_2_def")
        .invoice(InvoiceBuilder::new("EVS1").line("1"))
        .build();
    f.store.replace(&merge, true).unwrap();
    assert_eq!(f.store.read(fresh, None).unwrap().all_names(), ["EVS1"]);
}

#[test]
fn a_repeated_invoice_in_one_dataset_is_invalid_and_stores_nothing() {
    let f = fixture();
    let mut ds = three_invoices(f.cargue);
    ds.excluded.push(tessia::domain::dataset::Excluded {
        invoice: "EVS1".into(),
        reason: "dup".into(),
    });
    assert!(matches!(
        f.store.replace(&ds, false),
        Err(DatasetError::Invalid(_))
    ));
    assert!(matches!(
        f.store.read(f.cargue, None),
        Err(DatasetError::NotFound(_))
    ));
}

fn read_one(f: &Fixture, name: &str) -> Invoice {
    f.store
        .read_invoices(f.cargue, Some(&names(&[name])))
        .unwrap()
        .remove(0)
}

#[test]
fn update_invoices_rewrites_lines_keeps_position_and_bumps_the_version() {
    let f = fixture();
    f.store.replace(&three_invoices(f.cargue), false).unwrap();
    let mut a = read_one(&f, "EVS1");
    let mut b = read_one(&f, "EVS2");
    a.lines = vec![sample_line("EVS1", 1, "11"), sample_line("EVS1", 2, "12")];
    a.id_mipres = "m1".into();
    b.conciliacion = Some(InvoiceAudit {
        state: "OK".into(),
        ..InvoiceAudit::default()
    });
    f.store
        .update_invoices(f.cargue, &[(b.clone(), b.version), (a.clone(), a.version)])
        .expect("update");

    let read = f.store.read(f.cargue, None).unwrap();
    let names: Vec<&str> = read.invoices.iter().map(|i| i.invoice.as_str()).collect();
    assert_eq!(names, ["EVS1", "EVS2", "EVS5"], "position kept");
    assert_eq!(read.invoices[0].version, 1);
    assert_eq!(read.invoices[0].lines.len(), 2);
    assert_eq!(read.invoices[0].id_mipres, "m1");
    assert_eq!(read.invoices[1].version, 1);
    assert_eq!(read.invoices[1].conciliacion, b.conciliacion);
    assert_eq!(
        read.invoices[1].lines, b.lines,
        "lines rewritten identically"
    );
    assert_eq!(
        read.invoices[2].version, 0,
        "untouched invoice keeps its version"
    );
}

#[test]
fn a_stale_or_unknown_update_is_rejected_atomically() {
    let f = fixture();
    f.store.replace(&three_invoices(f.cargue), false).unwrap();
    let mut a = read_one(&f, "EVS1");
    let mut b = read_one(&f, "EVS2");
    a.lines[0].quantity = "changed".into();
    b.lines[0].quantity = "changed".into();

    // EVS2 is stale: nothing is written, not even EVS1's change.
    let err = f
        .store
        .update_invoices(
            f.cargue,
            &[(a.clone(), a.version), (b.clone(), b.version + 5)],
        )
        .unwrap_err();
    match &err {
        DatasetError::VersionConflict {
            invoice,
            stored,
            expected,
        } => {
            assert_eq!((invoice.as_str(), *stored, *expected), ("EVS2", 0, 5));
        }
        other => panic!("expected conflict, got {other:?}"),
    }
    assert!(err.is_retryable() && err.is_version_conflict());
    assert_eq!(read_one(&f, "EVS1").lines[0].quantity, "1");

    // Excluded / manual-review / unknown invoices are not updatable.
    for name in ["EVS3", "EVS4", "NOPE"] {
        let ghost = Invoice::new(name);
        assert!(
            matches!(
                f.store.update_invoices(f.cargue, &[(ghost, 0)]),
                Err(DatasetError::InvoiceNotFound { .. })
            ),
            "{name}"
        );
    }
    // The same invoice twice is a programming error.
    assert!(matches!(
        f.store
            .update_invoices(f.cargue, &[(a.clone(), 0), (a.clone(), 0)]),
        Err(DatasetError::Invalid(_))
    ));
    // Updating twice with the same stale version conflicts the second time.
    f.store
        .update_invoices(f.cargue, &[(a.clone(), a.version)])
        .unwrap();
    assert!(
        f.store
            .update_invoices(f.cargue, &[(a.clone(), a.version)])
            .unwrap_err()
            .is_version_conflict()
    );
}

#[test]
fn re_preparing_an_invoice_conflicts_with_a_reader_holding_the_old_version() {
    let f = fixture();
    f.store.replace(&three_invoices(f.cargue), false).unwrap();
    let held = read_one(&f, "EVS1"); // conciliar read it
    let again = DatasetBuilder::new(f.cargue)
        .load(LOAD)
        .invoice(InvoiceBuilder::new("EVS1").line("77"))
        .build();
    f.store.replace(&again, true).unwrap(); // preparar --invoices EVS1
    let err = f
        .store
        .update_invoices(f.cargue, &[(held.clone(), held.version)])
        .unwrap_err();
    assert!(err.is_version_conflict(), "{err:?}");
    // The fresh read works.
    let fresh = read_one(&f, "EVS1");
    assert_eq!(fresh.version, 1);
    f.store
        .update_invoices(f.cargue, &[(fresh.clone(), fresh.version)])
        .unwrap();
}

#[test]
fn concurrent_updates_of_the_same_invoice_have_one_winner() {
    let f = fixture();
    f.store.replace(&three_invoices(f.cargue), false).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|n| {
            let store = f.store.clone();
            let barrier = barrier.clone();
            let cargue = f.cargue;
            std::thread::spawn(move || {
                let mut inv = store
                    .read_invoices(cargue, Some(&["EVS1".to_string()]))
                    .unwrap()
                    .remove(0);
                inv.id_mipres = format!("writer-{n}");
                barrier.wait();
                let version = inv.version;
                store.update_invoices(cargue, &[(inv, version)])
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let ok = results.iter().filter(|r| r.is_ok()).count();
    let conflicts = results
        .iter()
        .filter(|r| matches!(r, Err(e) if e.is_version_conflict()))
        .count();
    assert_eq!((ok, conflicts), (1, 1), "{results:?}");
    assert_eq!(read_one(&f, "EVS1").version, 1);
}

#[test]
fn concurrent_updates_of_different_invoices_never_conflict() {
    let f = fixture();
    f.store.replace(&three_invoices(f.cargue), false).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["EVS1", "EVS2"]
        .into_iter()
        .map(|name| {
            let (store, barrier, cargue) = (f.store.clone(), barrier.clone(), f.cargue);
            std::thread::spawn(move || {
                let inv = store
                    .read_invoices(cargue, Some(&[name.to_string()]))
                    .unwrap()
                    .remove(0);
                barrier.wait();
                let version = inv.version;
                store.update_invoices(cargue, &[(inv, version)])
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap().expect("no conflict");
    }
}

fn prepared(cargue: i64, load: &str, invoice: &str, totals: &[&str]) -> Dataset {
    let mut inv = InvoiceBuilder::new(invoice);
    for t in totals {
        inv = inv.line(t);
    }
    DatasetBuilder::new(cargue).load(load).invoice(inv).build()
}

#[test]
fn agreement_total_sums_lines_and_normalizes_the_name() {
    let f = fixture();
    f.store
        .replace(
            &prepared(f.cargue, LOAD, "EVS1", &["1.234,50", "10", "0,25"]),
            false,
        )
        .unwrap();
    assert_eq!(
        f.store.agreement_total("evs-1").unwrap(),
        Some(dec("1244.75"))
    );
    assert_eq!(
        f.store
            .agreement_total("EVS1")
            .unwrap()
            .unwrap()
            .to_string(),
        "1244.75"
    );
    assert_eq!(f.store.agreement_total("EVS2").unwrap(), None);
}

#[test]
fn agreement_total_distinguishes_no_lines_excluded_and_unreadable() {
    let f = fixture();
    let ds = DatasetBuilder::new(f.cargue)
        .invoice(InvoiceBuilder::new("EVS1"))
        .excluded("EVS2", "X")
        .manual_review("EVS3", "C", "R")
        .invoice(InvoiceBuilder::new("EVS4").line("5").line("no es un monto"))
        .build();
    f.store.replace(&ds, false).unwrap();
    assert_eq!(
        f.store.agreement_total("EVS1").unwrap(),
        Some(Decimal::ZERO)
    );
    assert_eq!(f.store.agreement_total("EVS2").unwrap(), None);
    assert_eq!(f.store.agreement_total("EVS3").unwrap(), None);
    assert!(matches!(
        f.store.agreement_total("EVS4"),
        Err(DatasetError::Invalid(_))
    ));
}

#[test]
fn agreement_total_ties_go_to_the_highest_cargue_seq() {
    let f = fixture(); // cargue seq of LOAD
    let other = seed_cargue(&f.db.pool(), "Manual_5_abc", 'c');
    assert!(other > f.cargue);
    f.store
        .replace(&prepared(f.cargue, LOAD, "EVS1", &["10"]), false)
        .unwrap();
    f.store
        .replace(&prepared(other, "Manual_5_abc", "EVS1", &["20"]), false)
        .unwrap();
    assert_eq!(f.store.agreement_total("EVS1").unwrap(), Some(dec("20")));
    // Re-storing the older cargue does not change the tie-break.
    f.store
        .replace(&prepared(f.cargue, LOAD, "EVS1", &["11"]), false)
        .unwrap();
    assert_eq!(f.store.agreement_total("EVS1").unwrap(), Some(dec("20")));
    // The newer cargue holds it as EXCLUIDA: the older prepared one answers.
    let excluded = DatasetBuilder::new(other)
        .load("Manual_5_abc")
        .excluded("EVS1", "X")
        .build();
    f.store.replace(&excluded, false).unwrap();
    assert_eq!(f.store.agreement_total("EVS1").unwrap(), Some(dec("11")));
}

#[test]
fn an_unprepared_newer_upload_does_not_shadow_the_current_cargue() {
    let f = fixture();
    let old = f.cargue;
    f.store
        .replace(&prepared(old, LOAD, "EVS1", &["10"]), false)
        .unwrap();
    assert_eq!(f.store.current_cargue(LOAD).unwrap(), Some(old));

    // New bytes of the same load: ingested, not prepared yet.
    let newer = seed_cargue(&f.db.pool(), LOAD, 'b');
    assert!(newer > old);
    assert_eq!(
        f.store.current_cargue(LOAD).unwrap(),
        Some(old),
        "pointer keeps the prepared one"
    );
    assert_eq!(f.store.agreement_total("EVS1").unwrap(), Some(dec("10")));
    assert_eq!(
        f.store.read_current(LOAD, None).unwrap().map(|d| d.cargue),
        Some(old)
    );

    // Preparing it moves the pointer, atomically with the dataset.
    f.store
        .replace(&prepared(newer, LOAD, "EVS1", &["30"]), false)
        .unwrap();
    assert_eq!(f.store.current_cargue(LOAD).unwrap(), Some(newer));
    assert_eq!(f.store.agreement_total("EVS1").unwrap(), Some(dec("30")));

    // Preparing the older one again makes it current again.
    f.store
        .replace(&prepared(old, LOAD, "EVS1", &["10"]), false)
        .unwrap();
    assert_eq!(f.store.current_cargue(LOAD).unwrap(), Some(old));
    assert_eq!(f.store.agreement_total("EVS1").unwrap(), Some(dec("10")));
}

#[test]
fn a_load_without_a_pointer_resolves_to_its_newest_upload() {
    let f = fixture();
    assert_eq!(f.store.current_cargue(LOAD).unwrap(), Some(f.cargue));
    assert_eq!(f.store.current_cargue("Otro").unwrap(), None);
    assert_eq!(
        f.store.read_current(LOAD, None).unwrap(),
        None,
        "nothing prepared yet"
    );
    assert_eq!(f.store.read_current("Otro", None).unwrap(), None);
}

#[test]
fn a_failed_replace_leaves_the_previous_current_cargue_and_dataset() {
    let f = fixture();
    let old = f.cargue;
    f.store
        .replace(&prepared(old, LOAD, "EVS1", &["10"]), false)
        .unwrap();
    let newer = seed_cargue(&f.db.pool(), LOAD, 'b');
    let mut bad = prepared(newer, LOAD, "EVS1", &["30"]);
    bad.excluded.push(tessia::domain::dataset::Excluded {
        invoice: "EVS1".into(),
        reason: "dup".into(),
    });
    assert!(f.store.replace(&bad, false).is_err());
    assert_eq!(f.store.current_cargue(LOAD).unwrap(), Some(old));
    assert!(matches!(
        f.store.read(newer, None),
        Err(DatasetError::NotFound(_))
    ));
}

#[test]
fn save_preparation_writes_the_header_without_touching_invoices() {
    let f = fixture();
    let mut ds = prepared(f.cargue, LOAD, "EVS1", &["10"]);
    ds.preparation.nt_version = "v1".into();
    f.store.replace(&ds, false).unwrap();

    let mut header = f.store.preparation(f.cargue).unwrap().expect("header");
    header.nt_version = "v2".into();
    header.max_amount = Some(dec("250000"));
    header.automatic_subplans = names(&["EVENTO POS"]);
    header.stats.evaluated = 3;
    header.skipped_lines = vec![8];
    f.store.save_preparation(f.cargue, &header).unwrap();

    let back = f.store.read(f.cargue, None).unwrap();
    assert_eq!(back.preparation, header);
    assert_eq!(back.invoices.len(), 1);
    assert_eq!(back.invoices[0].version, 0);

    // A header alone (no invoices yet) is a readable, empty dataset.
    let fresh = seed_cargue(&f.db.pool(), "Manual_9_x", 'd');
    header.load.id = "Manual_9_x".into();
    f.store.save_preparation(fresh, &header).unwrap();
    assert!(f.store.read(fresh, None).unwrap().invoices.is_empty());
    assert_eq!(f.store.current_cargue("Manual_9_x").unwrap(), Some(fresh));
    assert!(matches!(
        f.store.save_preparation(fresh + 100, &header),
        Err(DatasetError::CargueNotFound(_))
    ));
}

#[test]
fn cargues_do_not_see_each_others_invoices() {
    // Two cargues of one database do not see each other's invoices.
    let f = fixture();
    let other = seed_cargue(&f.db.pool(), "Manual_5_abc", 'c');
    f.store
        .replace(&prepared(f.cargue, LOAD, "EVS1", &["10"]), false)
        .unwrap();
    f.store
        .replace(
            &prepared(other, "Manual_5_abc", "EVS1", &["20", "1"]),
            false,
        )
        .unwrap();
    assert_eq!(
        f.store.read(f.cargue, None).unwrap().invoices[0]
            .lines
            .len(),
        1
    );
    assert_eq!(
        f.store.read(other, None).unwrap().invoices[0].lines.len(),
        2
    );
    f.store
        .replace(&prepared(f.cargue, LOAD, "EVS7", &["1"]), false)
        .unwrap();
    assert_eq!(f.store.read(other, None).unwrap().all_names(), ["EVS1"]);
}

#[test]
fn the_context_exposes_the_store() {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    let ingested = ctx
        .cargues()
        .ingest(&NewCargue::from_load(
            &tessia::domain::cargue::load_context("Convenio_01102026_02102026.csv"),
            &"e".repeat(64),
        ))
        .unwrap();
    let ds = prepared(
        ingested.cargue.seq,
        "Convenio_01102026_02102026",
        "EVS1",
        &["3"],
    );
    ctx.dataset().replace(&ds, false).unwrap();
    let again = CarguesStore::new(db.pool())
        .newest("Convenio_01102026_02102026")
        .unwrap()
        .unwrap();
    assert_eq!(ctx.dataset().invoice_names(again.seq).unwrap(), ["EVS1"]);
}
