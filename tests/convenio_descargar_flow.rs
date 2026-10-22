//! `convenio descargar` against the local fake Medicar portal and a throwaway
//! Postgres database. No real system is contacted.

use std::time::Duration;

use chrono::NaiveDate;
use sha2::{Digest, Sha256};

use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::cargue::LoadOrigin;
use tessia::phases::convenio_descargar::mock::{self, Link, Login, MockMedicar};
use tessia::phases::convenio_descargar::{
    Args, DownloadStatus, download_range, download_range_with, run,
};

const ID: &str = "Convenio_21102026_21102026";
const FILE: &str = "Convenio_21102026_21102026.csv";
const LEASE: &str = "convenio_descargar:Convenio_21102026_21102026";

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 21).expect("date")
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn setup() -> (TestDb, AppContext, MockMedicar) {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    (db, ctx, MockMedicar::start())
}

fn workdir_entries(ctx: &AppContext) -> usize {
    std::fs::read_dir(&ctx.workdir_root)
        .map(|entries| entries.count())
        .unwrap_or(0)
}

fn assert_nothing_published(ctx: &AppContext) {
    assert!(ctx.cargues().newest(ID).expect("newest").is_none());
    assert!(ctx.artifacts().list(ID).expect("list").is_empty());
    assert_eq!(workdir_entries(ctx), 0, "workdir must be removed");
    assert_eq!(ctx.leases().holder(LEASE).expect("holder"), None);
}

fn download(
    ctx: &AppContext,
    portal: &MockMedicar,
) -> anyhow::Result<tessia::phases::convenio_descargar::DownloadOutcome> {
    download_range_with(ctx, day(), day(), portal.options())
}

#[test]
fn full_flow_publishes_the_csv_as_a_medicar_cargue() {
    let (_db, ctx, portal) = setup();
    let csv = b"Factura|Plu\nFEV1|7\n";
    portal.set_csv(csv);
    let out = download(&ctx, &portal).expect("download");

    assert_eq!(out.cargue_id, ID);
    assert_eq!(out.file_name, FILE);
    assert_eq!(out.object_key, format!("{ID}__{FILE}"));
    assert_eq!(out.sha256, sha(csv));
    assert_eq!(out.size, csv.len() as u64);
    assert_eq!(out.status, DownloadStatus::Created);

    let cargue = ctx.cargues().newest(ID).expect("newest").expect("cargue");
    assert_eq!(cargue.sha256, sha(csv));
    assert_eq!(cargue.origin(), LoadOrigin::Medicar);
    assert_eq!(cargue.name, FILE);
    assert_eq!(cargue.date_from, Some(day()));
    assert_eq!(cargue.date_to, Some(day()));
    assert_eq!(ctx.artifacts().fetch_bytes(ID, FILE).expect("fetch"), csv);
    assert_eq!(workdir_entries(&ctx), 0);
    assert_eq!(ctx.leases().holder(LEASE).expect("holder"), None);
}

#[test]
fn the_flow_sends_the_expected_requests_in_order() {
    let (_db, ctx, portal) = setup();
    download(&ctx, &portal).expect("download");
    let requests = portal.requests();
    let steps: Vec<(String, String)> = requests
        .iter()
        .map(|r| (r.method.clone(), r.path.clone()))
        .collect();
    let base = mock::BASE_PATH;
    let products = mock::PRODUCTS_PATH;
    let expected = [
        ("GET", format!("{base}/")),
        ("POST", format!("{base}/procesar.php")),
        ("GET", format!("{products}/convenioArticulos.php")),
        (
            "POST",
            format!("{products}/exportarExcelConvenioArticulos.php"),
        ),
        ("GET", format!("{products}/{}", mock::CSV_PATH)),
    ];
    let expected: Vec<(String, String)> = expected
        .into_iter()
        .map(|(m, p)| (m.to_owned(), p))
        .collect();
    assert_eq!(steps, expected);

    for request in &requests {
        assert!(
            request
                .header("user-agent")
                .is_some_and(|ua| ua.contains("Chrome/128"))
        );
    }
    let login = requests[1].body_text();
    for part in [
        "f_id=ID-SINT".to_owned(),
        format!("f_log={}", hex::encode(md5::Md5::digest(mock::USERNAME))),
        format!("f_pass={}", hex::encode(md5::Md5::digest(mock::PASSWORD))),
        "f_pass_nuevo=".to_owned(),
        "f_pass_confirma=".to_owned(),
        "csrf_token=tok%26sint".to_owned(),
    ] {
        assert!(login.contains(&part), "{part} in {login}");
    }
    assert_eq!(
        requests[1].header("x-requested-with"),
        Some("XMLHttpRequest")
    );
    assert!(
        requests[3]
            .header("content-type")
            .is_some_and(|c| c.starts_with("multipart/form-data; boundary="))
    );
}

#[test]
fn the_export_form_is_the_exact_ordered_field_list() {
    let (_db, ctx, portal) = setup();
    let from = NaiveDate::from_ymd_opt(2026, 10, 1).expect("date");
    download_range_with(&ctx, from, day(), portal.options()).expect("download");
    let export = portal
        .requests()
        .into_iter()
        .find(|r| r.path.ends_with("exportarExcelConvenioArticulos.php"))
        .expect("export request");
    let body = export.body_text();
    let marker = "Content-Disposition: form-data; name=\"";
    let fields: Vec<String> = body
        .split(marker)
        .skip(1)
        .map(|chunk| {
            let (name, rest) = chunk.split_once("\"\r\n\r\n").expect("field header");
            let (value, _) = rest.split_once("\r\n--").expect("field end");
            format!("{name}={value}")
        })
        .collect();
    let golden = include_str!("fixtures/convenio_descargar/export_form.txt");
    let want: Vec<&str> = golden.lines().collect();
    assert_eq!(want.len(), 77);
    assert_eq!(fields, want);
}

#[test]
fn a_relative_csv_link_is_resolved_against_the_products_base() {
    let (_db, ctx, portal) = setup();
    portal.set_link(Link::Relative);
    let out = download(&ctx, &portal).expect("download");
    assert_eq!(out.status, DownloadStatus::Created);
    assert_eq!(portal.count("GET", mock::CSV_PATH), 1);
}

#[test]
fn a_rejected_login_is_permanent_and_publishes_nothing() {
    let (_db, ctx, portal) = setup();
    portal.set_login(Login::Rejected);
    let err = download(&ctx, &portal).expect_err("rejected");
    assert!(
        err.to_string()
            .contains("login was not accepted (HTTP 200)"),
        "{err}"
    );
    assert_eq!(portal.count("GET", "/"), 1, "no whole-flow retry");
    assert_eq!(portal.count("POST", "procesar.php"), 1);
    assert_nothing_published(&ctx);
}

#[test]
fn a_login_page_without_csrf_token_is_permanent() {
    let (_db, ctx, portal) = setup();
    portal.set_login(Login::NoCsrf);
    let err = download(&ctx, &portal).expect_err("no csrf");
    assert!(err.to_string().contains("csrf_token not found"), "{err}");
    assert_eq!(portal.count("GET", "/"), 1);
    assert_eq!(portal.count("POST", "procesar.php"), 0);
    assert_nothing_published(&ctx);
}

#[test]
fn an_export_without_link_is_permanent_and_leaves_no_partial_artifact() {
    let (_db, ctx, portal) = setup();
    portal.set_link(Link::Missing);
    let err = download(&ctx, &portal).expect_err("no link");
    assert!(
        err.to_string()
            .contains("no CSV link (no data for the range, or the portal changed)"),
        "{err}"
    );
    assert_eq!(portal.count("GET", "/"), 1);
    assert_eq!(portal.count("GET", mock::CSV_PATH), 0);
    assert_nothing_published(&ctx);
}

#[test]
fn a_transient_error_restarts_the_whole_flow_with_a_fresh_session() {
    let (_db, ctx, portal) = setup();
    portal.fail("exportarExcelConvenioArticulos.php", 503, 1);
    let out = download(&ctx, &portal).expect("second attempt succeeds");
    assert_eq!(out.status, DownloadStatus::Created);
    assert_eq!(portal.count("GET", "/"), 2);
    assert_eq!(portal.count("POST", "procesar.php"), 2);
    let starts: Vec<_> = portal
        .requests()
        .into_iter()
        .filter(|r| r.method == "GET" && r.path == format!("{}/", mock::BASE_PATH))
        .collect();
    assert!(
        starts.iter().all(|r| r.header("cookie").is_none()),
        "each attempt starts with an empty cookie jar"
    );
}

#[test]
fn the_login_request_is_retried_on_5xx_without_restarting_the_flow() {
    let (_db, ctx, portal) = setup();
    portal.fail("procesar.php", 502, 2);
    download(&ctx, &portal).expect("download");
    assert_eq!(portal.count("POST", "procesar.php"), 3);
    assert_eq!(portal.count("GET", "/"), 1);
}

#[test]
fn a_persistent_transient_error_stops_after_three_attempts() {
    let (_db, ctx, portal) = setup();
    portal.fail("convenioArticulos.php", 500, 99);
    let err = download(&ctx, &portal).expect_err("exhausted");
    assert!(err.to_string().contains("HTTP 500"), "{err}");
    assert_eq!(portal.count("GET", "/"), 3);
    assert_nothing_published(&ctx);
}

#[test]
fn a_permanent_4xx_is_not_retried() {
    let (_db, ctx, portal) = setup();
    portal.fail("convenioArticulos.php", 403, 99);
    let err = download(&ctx, &portal).expect_err("forbidden");
    assert!(err.to_string().contains("HTTP 403"), "{err}");
    assert_eq!(portal.count("GET", "/"), 1);
    assert_nothing_published(&ctx);
}

#[test]
fn a_failing_csv_download_leaves_no_partial_artifact() {
    let (_db, ctx, portal) = setup();
    portal.fail(mock::CSV_PATH, 404, 99);
    let err = download(&ctx, &portal).expect_err("missing csv");
    assert!(err.to_string().contains("download CSV: HTTP 404"), "{err}");
    assert_eq!(portal.count("GET", mock::CSV_PATH), 1);
    assert_nothing_published(&ctx);

    let (_db, ctx, portal) = setup();
    portal.fail(mock::CSV_PATH, 503, 1);
    download(&ctx, &portal).expect("retry after a transient csv failure");
    assert_eq!(portal.count("GET", mock::CSV_PATH), 2);
}

#[test]
fn the_same_bytes_are_unchanged_and_different_bytes_are_a_new_upload() {
    let (_db, ctx, portal) = setup();
    let first = b"Factura|Plu\nFEV1|7\n";
    portal.set_csv(first);
    assert_eq!(
        download(&ctx, &portal).expect("first").status,
        DownloadStatus::Created
    );

    let again = download(&ctx, &portal).expect("again");
    assert_eq!(again.status, DownloadStatus::Unchanged);
    let seq = ctx.cargues().newest(ID).expect("newest").expect("row").seq;

    let second = b"Factura|Plu\nFEV1|7\nFEV2|8\n";
    portal.set_csv(second);
    let changed = download(&ctx, &portal).expect("changed");
    assert_eq!(changed.status, DownloadStatus::NewUpload);
    assert_eq!(changed.sha256, sha(second));
    let newest = ctx.cargues().newest(ID).expect("newest").expect("row");
    assert!(newest.seq > seq);
    assert_eq!(newest.sha256, sha(second));
    assert!(
        ctx.cargues().get(ID, &sha(first)).expect("get").is_some(),
        "old upload kept"
    );
    assert_eq!(
        ctx.artifacts().fetch_bytes(ID, FILE).expect("fetch"),
        second
    );
    assert_eq!(ctx.artifacts().list(ID).expect("list").len(), 1);
}

#[test]
fn a_held_lease_blocks_the_download_before_any_request() {
    let (_db, ctx, portal) = setup();
    assert!(
        ctx.leases()
            .acquire(LEASE, "otra-maquina", Duration::from_secs(60))
            .expect("other machine")
    );
    let err = download(&ctx, &portal).expect_err("held");
    let text = err.to_string();
    assert!(text.contains("otra máquina ya está descargando"), "{text}");
    assert!(text.contains(ID) && text.contains("otra-maquina"), "{text}");
    assert!(portal.requests().is_empty());
    assert_eq!(
        ctx.leases().holder(LEASE).expect("holder").as_deref(),
        Some("otra-maquina")
    );

    ctx.leases()
        .release(LEASE, "otra-maquina")
        .expect("release");
    assert_eq!(
        download(&ctx, &portal).expect("free").status,
        DownloadStatus::Created
    );
}

#[test]
fn an_expired_lease_does_not_block() {
    let (_db, ctx, portal) = setup();
    ctx.leases()
        .acquire(LEASE, "otra-maquina", Duration::from_millis(30))
        .expect("short lease");
    std::thread::sleep(Duration::from_millis(100));
    download(&ctx, &portal).expect("expired lease is taken over");
}

#[test]
fn end_before_start_is_rejected_before_any_request() {
    let (_db, ctx, portal) = setup();
    let later = NaiveDate::from_ymd_opt(2026, 10, 22).expect("date");
    let err = download_range_with(&ctx, later, day(), portal.options()).expect_err("order");
    assert!(
        err.to_string()
            .contains("end date is before the start date"),
        "{err}"
    );
    assert!(portal.requests().is_empty());
}

fn configured(ctx: AppContext, portal: &MockMedicar) -> AppContext {
    let settings = ctx
        .settings
        .clone()
        .with("medicar.base_url", &portal.base_url())
        .with("medicar.products_url", &portal.products_url())
        .with("medicar.identifier", mock::IDENTIFIER)
        .with("medicar.username", mock::USERNAME)
        .with("medicar.password", mock::PASSWORD);
    ctx.with_settings(settings)
}

fn args(desde: Option<&str>, hasta: Option<&str>, actual: bool) -> Args {
    Args {
        desde: desde.map(str::to_owned),
        hasta: hasta.map(str::to_owned),
        actual,
    }
}

#[test]
fn the_command_uses_the_medicar_settings() {
    let (_db, ctx, portal) = setup();
    let ctx = configured(ctx, &portal);
    run(&ctx, args(Some("2026-10-21"), Some("2026-10-21"), false)).expect("run");
    assert!(ctx.artifacts().is_published(ID, FILE).expect("published"));
    assert_eq!(
        portal.count("POST", "exportarExcelConvenioArticulos.php"),
        1
    );
}

#[test]
fn missing_medicar_settings_name_the_keys() {
    let (_db, ctx, _portal) = setup();
    let err = download_range(&ctx, day(), day()).expect_err("unconfigured");
    let text = format!("{err:#}");
    for key in [
        "medicar.base_url (TESSIA_MEDICAR_BASE_URL)",
        "medicar.password (TESSIA_MEDICAR_PASSWORD)",
    ] {
        assert!(text.contains(key), "{key} in {text}");
    }
}

#[test]
fn date_flag_errors_come_before_any_request_or_config_check() {
    let (_db, ctx, portal) = setup();
    let ctx = configured(ctx, &portal);
    let cases = [
        (
            args(Some("2026-10-01"), None, true),
            "use either --actual or --desde/--hasta",
        ),
        (args(None, None, false), "--desde must be YYYY-MM-DD"),
        (
            args(Some("2026-10-01"), Some("21/10/2026"), false),
            "--hasta must be YYYY-MM-DD",
        ),
        (
            args(Some("2026-10-05"), Some("2026-10-04"), false),
            "--hasta is before --desde",
        ),
    ];
    for (case, message) in cases {
        let err = run(&ctx, case).expect_err(message);
        assert_eq!(err.to_string(), message);
    }
    assert!(portal.requests().is_empty());
    assert_eq!(ctx.cargues().newest(ID).expect("newest"), None);
}

#[test]
fn actual_downloads_the_current_month_through_today() {
    let (_db, ctx, portal) = setup();
    let ctx = configured(ctx, &portal);
    run(&ctx, args(None, None, true)).expect("run --actual");
    let export = portal
        .requests()
        .into_iter()
        .find(|r| r.path.ends_with("exportarExcelConvenioArticulos.php"))
        .expect("export");
    let body = export.body_text();
    let today = chrono::Local::now().date_naive();
    let first = today.format("%Y-%m-01").to_string();
    assert!(
        body.contains(&format!("name=\"f_fecha\"\r\n\r\n{first}\r\n")),
        "{body}"
    );
    assert!(
        body.contains(&format!(
            "name=\"f_fecha_final\"\r\n\r\n{}\r\n",
            today.format("%Y-%m-%d")
        )),
        "{body}"
    );
}
