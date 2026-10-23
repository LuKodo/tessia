use std::collections::HashSet;
use std::sync::{Arc, Barrier};
use std::thread;

use super::*;
use crate::db::test_support::TestDb;

fn opts(owner: &str, ttl: Duration) -> ClaimOptions {
    ClaimOptions::new(owner, Some("C1"), ttl)
}

fn names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("FEV{i:03}")).collect()
}

fn items(store: &ProcesoStore, phase: Phase, invoices: &[String]) -> Vec<ClaimItem> {
    let loaded = store.load(phase, Some(invoices)).expect("load");
    invoices
        .iter()
        .map(|i| ClaimItem::from_loaded(i, loaded.get(i)))
        .collect()
}

fn save_row(invoice: &str, phase: Phase, outcome: Outcome, intentos: i32) -> SaveRow {
    SaveRow {
        invoice: invoice.into(),
        phase,
        outcome,
        intentos,
        cargue_id: None,
    }
}

#[test]
fn claim_by_insert_then_outcome_roundtrip() {
    let db = TestDb::new();
    let store = ProcesoStore::new(db.pool());
    let inv = names(1);
    let claim = opts("a", Duration::from_secs(60));
    let got = store
        .claim_available(Phase::Rips, &items(&store, Phase::Rips, &inv), &claim)
        .expect("claim");
    assert_eq!(got.get("FEV000"), Some(&0));
    let loaded = store.load(Phase::Rips, None).expect("load");
    let row = &loaded["FEV000"];
    assert_eq!(
        (
            row.state,
            row.claimed_by.as_deref(),
            row.cargue_id.as_deref()
        ),
        (PhaseState::EnProgreso, Some("a"), Some("C1"))
    );

    assert!(
        store
            .save_outcome(Phase::Rips, "FEV000", &Outcome::failed("RIPS_ERROR"), 0)
            .expect("outcome")
    );
    let row = &store.load(Phase::Rips, None).expect("load")["FEV000"];
    assert_eq!(
        (row.state, row.motivo.as_str(), row.intentos, row.version),
        (PhaseState::Failed, "RIPS_ERROR", 1, 1)
    );
    assert!(row.claimed_by.is_none() && row.claimed_at.is_none());
    // The same claim version cannot save twice.
    assert!(
        !store
            .save_outcome(Phase::Rips, "FEV000", &Outcome::done(), 0)
            .expect("stale")
    );
}

#[test]
fn a_second_claim_of_the_same_unclaimed_invoice_loses() {
    let db = TestDb::new();
    let store = ProcesoStore::new(db.pool());
    let inv = names(2);
    let both_loaded_nothing = items(&store, Phase::Cuv, &inv);
    let a = store
        .claim_available(
            Phase::Cuv,
            &both_loaded_nothing,
            &opts("a", Duration::from_secs(60)),
        )
        .expect("a");
    let b = store
        .claim_available(
            Phase::Cuv,
            &both_loaded_nothing,
            &opts("b", Duration::from_secs(60)),
        )
        .expect("b");
    assert_eq!(a.len(), 2);
    assert!(b.is_empty());
}

#[test]
fn claim_over_a_finished_row_is_compare_and_set() {
    let db = TestDb::new();
    let store = ProcesoStore::new(db.pool());
    store
        .save(&save_row("FEV000", Phase::Soporte, Outcome::failed("X"), 1))
        .expect("seed");
    let loaded = store.load(Phase::Soporte, None).expect("load");
    let item = ClaimItem::from_loaded("FEV000", loaded.get("FEV000"));
    // Someone else changes the row after we loaded it.
    store
        .save(&save_row("FEV000", Phase::Soporte, Outcome::done(), 2))
        .expect("concurrent change");
    let got = store
        .claim_available(Phase::Soporte, &[item], &opts("a", Duration::from_secs(60)))
        .expect("claim");
    assert!(got.is_empty(), "the stale expectation must lose");
    // With a fresh load the claim succeeds and moves the version on.
    let fresh = items(&store, Phase::Soporte, &names(1));
    let got = store
        .claim_available(Phase::Soporte, &fresh, &opts("a", Duration::from_secs(60)))
        .expect("claim");
    assert_eq!(got.get("FEV000"), Some(&(loaded["FEV000"].version + 2)));
}

#[test]
fn start_all_is_atomic() {
    let db = TestDb::new();
    let store = ProcesoStore::new(db.pool());
    let inv = names(3);
    let claim = opts("a", Duration::from_secs(60));
    let stale_view = items(&store, Phase::Rips, &inv);
    // Another machine takes the last invoice first.
    let other = store
        .claim_available(
            Phase::Rips,
            &stale_view[2..],
            &opts("b", Duration::from_secs(60)),
        )
        .expect("b");
    assert_eq!(other.len(), 1);
    assert!(
        store
            .start_all(Phase::Rips, &stale_view, &claim)
            .expect("start_all")
            .is_none()
    );
    let rows = store.load(Phase::Rips, None).expect("load");
    assert_eq!(rows.len(), 1, "nothing of the rejected batch is retained");
    // Without the contested invoice the batch is claimed whole.
    let ok = store
        .start_all(Phase::Rips, &stale_view[..2], &claim)
        .expect("start_all")
        .expect("claimed");
    assert_eq!(ok.len(), 2);
}

#[test]
fn stale_claims_are_taken_over_and_the_old_owner_is_ignored() {
    let db = TestDb::new();
    let store = ProcesoStore::new(db.pool());
    let inv = names(1);
    let first = store
        .claim_available(
            Phase::Derechos,
            &items(&store, Phase::Derechos, &inv),
            &opts("dead", Duration::from_secs(60)),
        )
        .expect("first");
    // Not stale yet: cannot take over.
    let live_view = items(&store, Phase::Derechos, &inv);
    let denied = store
        .claim_available(
            Phase::Derechos,
            &live_view,
            &opts("b", Duration::from_secs(3600)),
        )
        .expect("denied");
    assert!(denied.is_empty());
    thread::sleep(Duration::from_millis(30));
    let taken = store
        .claim_available(
            Phase::Derechos,
            &live_view,
            &opts("b", Duration::from_millis(10)),
        )
        .expect("takeover");
    assert_eq!(taken.get("FEV000"), Some(&(first["FEV000"] + 1)));
    // The dead worker wakes up: its result is ignored.
    assert!(
        !store
            .save_outcome(Phase::Derechos, "FEV000", &Outcome::done(), first["FEV000"])
            .expect("late")
    );
    assert!(
        store
            .save_outcome(Phase::Derechos, "FEV000", &Outcome::done(), taken["FEV000"])
            .expect("new owner")
    );
    let row = &store.load(Phase::Derechos, None).expect("load")["FEV000"];
    assert_eq!((row.state, row.intentos), (PhaseState::Done, 1));
}

#[test]
fn release_restores_the_previous_state_or_deletes() {
    let db = TestDb::new();
    let store = ProcesoStore::new(db.pool());
    store
        .save(&save_row("FEV000", Phase::Cuv, Outcome::failed("OLD"), 3))
        .expect("seed");
    let inv = names(2);
    let previous = store.load(Phase::Cuv, Some(&inv)).expect("previous");
    let versions = store
        .claim_available(
            Phase::Cuv,
            &items(&store, Phase::Cuv, &inv),
            &opts("a", Duration::from_secs(60)),
        )
        .expect("claim");
    assert_eq!(
        store
            .release(Phase::Cuv, &versions, &previous)
            .expect("release"),
        2
    );
    let rows = store.load(Phase::Cuv, None).expect("load");
    assert_eq!(rows.len(), 1);
    let row = &rows["FEV000"];
    assert_eq!(
        (row.state, row.motivo.as_str(), row.intentos),
        (PhaseState::Failed, "OLD", 3)
    );
    // A claim that changed hands is not restored.
    assert_eq!(
        store
            .release(Phase::Cuv, &versions, &previous)
            .expect("again"),
        0
    );
}

#[test]
fn record_continues_the_attempts_counter() {
    let db = TestDb::new();
    let store = ProcesoStore::new(db.pool());
    let pairs = vec![
        ("FEV000".to_string(), Outcome::failed("A")),
        ("FEV001".to_string(), Outcome::done()),
    ];
    store
        .record(Phase::Rips, &HashMap::new(), &pairs, Some("C1"))
        .expect("first");
    let prev = store.load(Phase::Rips, None).expect("prev");
    store
        .record(Phase::Rips, &prev, &pairs[..1], Some("C1"))
        .expect("second");
    let rows = store.load(Phase::Rips, None).expect("rows");
    assert_eq!((rows["FEV000"].intentos, rows["FEV001"].intentos), (2, 1));
    assert_eq!(
        store
            .list_by_cargue("C1", Some(Phase::Rips))
            .expect("by cargue")
            .len(),
        2
    );
    assert!(
        store
            .list_by_cargue("C1", Some(Phase::Cuv))
            .expect("other phase")
            .is_empty()
    );
}

#[test]
fn insert_outcome_and_save_if_current_never_overwrite_newer_state() {
    let db = TestDb::new();
    let store = ProcesoStore::new(db.pool());
    assert!(
        store
            .insert_outcome(&save_row("FEV000", Phase::Rips, Outcome::done(), 1))
            .expect("insert")
    );
    assert!(
        !store
            .insert_outcome(&save_row("FEV000", Phase::Rips, Outcome::failed("X"), 1))
            .expect("exists")
    );
    let loaded = store.load(Phase::Rips, None).expect("load")["FEV000"].clone();
    let change = save_row("FEV000", Phase::Rips, Outcome::manual_review("M"), 2);
    assert!(store.save_if_current(&change, &loaded).expect("current"));
    assert!(!store.save_if_current(&change, &loaded).expect("now stale"));
}

#[test]
fn many_threads_never_claim_the_same_invoice_and_cover_all_of_them() {
    let db = TestDb::new();
    let store = ProcesoStore::new(db.pool());
    let invoices = names(60);
    // Every worker loaded the same (empty) state before racing.
    let view = items(&store, Phase::Rips, &invoices);
    let workers = 6;
    let barrier = Arc::new(Barrier::new(workers));
    let handles: Vec<_> = (0..workers)
        .map(|n| {
            let (store, view, barrier) = (store.clone(), view.clone(), barrier.clone());
            thread::spawn(move || {
                barrier.wait();
                let claim = opts(&format!("w{n}"), Duration::from_secs(60));
                store
                    .claim_available(Phase::Rips, &view, &claim)
                    .expect("claim")
            })
        })
        .collect();
    let mut seen = HashSet::new();
    for handle in handles {
        for invoice in handle.join().expect("worker").into_keys() {
            assert!(seen.insert(invoice.clone()), "{invoice} was claimed twice");
        }
    }
    assert_eq!(
        seen.len(),
        invoices.len(),
        "every invoice is claimed by someone"
    );
    let owners: HashSet<String> = store
        .load(Phase::Rips, None)
        .expect("load")
        .into_values()
        .filter_map(|s| s.claimed_by)
        .collect();
    assert!(!owners.is_empty());
}

#[test]
fn start_all_races_have_a_single_winner_per_invoice() {
    let db = TestDb::new();
    let store = ProcesoStore::new(db.pool());
    let invoices = names(20);
    let view = items(&store, Phase::Cuv, &invoices);
    let barrier = Arc::new(Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|n| {
            let (store, view, barrier) = (store.clone(), view.clone(), barrier.clone());
            thread::spawn(move || {
                barrier.wait();
                store
                    .start_all(
                        Phase::Cuv,
                        &view,
                        &opts(&format!("w{n}"), Duration::from_secs(60)),
                    )
                    .expect("start_all")
            })
        })
        .collect();
    let winners = handles
        .into_iter()
        .filter_map(|h| h.join().expect("worker"))
        .count();
    assert_eq!(winners, 1, "exactly one owner claims the whole batch");
    let owners: HashSet<String> = store
        .load(Phase::Cuv, None)
        .expect("load")
        .into_values()
        .filter_map(|s| s.claimed_by)
        .collect();
    assert_eq!(owners.len(), 1);
}
