//! The foundation pieces working together, from an integration test (proves
//! `TestDb` and the mock server are usable outside the crate).

use std::sync::Arc;

use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::{Outcome, Phase, PhaseState};
use tessia::nt::{MemoryNt, NtRow};
use tessia::phases::common::PhaseReport;
use tessia::storage::{CassandraStore, ObjectStore, layout};
use tessia::stores::proceso::ClaimItem;
use tessia::testing::{MockResponse, MockServer};

#[test]
fn a_phase_can_claim_work_publish_artifacts_and_report() {
    let db = TestDb::new();
    let row = NtRow {
        cum_facturar: Some("A-1".into()),
        ..NtRow::default()
    };
    let ctx = AppContext::for_tests(&db).with_nt(Arc::new(
        MemoryNt::new().with_version("v1", vec![("1".into(), row)]),
    ));
    let cargue = "Convenio_21102026_21102026";

    // Claim
    let invoices = ["FEV1".to_string(), "FEV2".to_string()];
    let items: Vec<ClaimItem> = invoices.iter().map(|i| ClaimItem::new(i)).collect();
    let claimed = ctx
        .proceso()
        .claim_available(Phase::Rips, &items, &ctx.claim_options(cargue))
        .expect("claim");
    assert_eq!(claimed.len(), 2);

    // Work in a temp workdir, publish, store outcomes
    let wd = ctx.workdir(cargue).expect("workdir");
    let mut report = PhaseReport::default();
    for (invoice, version) in &claimed {
        wd.write(&format!("{}/{invoice}.xml", layout::XML), b"<xml/>")
            .expect("write");
        let stored = ctx
            .proceso()
            .save_outcome(Phase::Rips, invoice, &Outcome::done(), *version)
            .expect("save");
        assert!(stored);
        report.push(invoice, Outcome::done());
    }
    ctx.artifacts()
        .publish_workdir(&wd, Some(layout::XML))
        .expect("publish");
    assert_eq!(report.counters().done, 2);

    // A second machine sees the finished state and the artifacts.
    let states = ctx.proceso().list_by_cargue(cargue, None).expect("states");
    assert!(states.iter().all(|s| s.state == PhaseState::Done));
    assert_eq!(ctx.artifacts().list(cargue).expect("artifacts").len(), 2);
    let nt = ctx.nt.lookup("v1", "cum_facturar", "A-1").expect("nt");
    assert_eq!(nt.len(), 1);
}

#[test]
fn the_cassandra_store_talks_to_a_mock_server_from_outside_the_crate() {
    let server = MockServer::start(|_| MockResponse::json(201, "{}"));
    let store = CassandraStore::new(server.url(), "bucket", "key").expect("store");
    store.put_bytes("a", b"x", "text/plain").expect("put");
    assert_eq!(server.requests().len(), 1);
}
