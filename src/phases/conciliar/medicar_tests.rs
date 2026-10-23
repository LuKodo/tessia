//! Tests of the Medicar formula client against the fake portal (ports of Go
//! `formula_test.go` and `unavailable_test.go`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rust_decimal::Decimal;

use super::fake_portal::{BASE_PATH, FakeFormula, FakePortal, IDENTIFIER, PASSWORD, USERNAME};
use super::medicar::{FormulaClient, MedicarError, MedicarOptions, Outage, formula_retry_policy};
use super::portal_html::Formula;
use crate::http::RetryPolicy;
use crate::testing::{MockResponse, MockServer};

const ARTICLE: &str = "ACETAMINOFEN/CODEINA 325+8 MG TABLETA";

/// A retry policy that records its backoffs instead of sleeping.
fn recording_policy() -> (RetryPolicy, Arc<Mutex<Vec<Duration>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = log.clone();
    let policy = RetryPolicy {
        jitter: 0.0,
        sleeper: Some(Arc::new(move |d| sink.lock().expect("log").push(d))),
        ..formula_retry_policy()
    };
    (policy, log)
}

fn options(base: &str, policy: RetryPolicy) -> MedicarOptions {
    let mut o = MedicarOptions::new(
        &format!("{base}{BASE_PATH}"),
        IDENTIFIER,
        USERNAME,
        PASSWORD,
    );
    o.retry = policy;
    o.read_timeout = Duration::from_secs(5);
    o
}

fn client(base: &str, policy: RetryPolicy) -> FormulaClient {
    FormulaClient::new(options(base, policy)).expect("client")
}

fn count(server: &MockServer, method: &str, path: &str) -> usize {
    let full = format!("{BASE_PATH}{path}");
    server
        .requests()
        .iter()
        .filter(|r| r.method == method && r.path.split('?').next() == Some(full.as_str()))
        .count()
}

fn lookup(c: &FormulaClient) -> Result<Formula, MedicarError> {
    c.lookup("900001", "7770001", "FVS9000001", ARTICLE)
}

fn portal() -> MockServer {
    FakePortal::new(vec![FakeFormula::sample()]).start()
}

/// A front that breaks the first `n` requests of each `METHOD path` in `drop`
/// with `status`, and forwards the rest to the portal.
fn broken_front(
    drop: &[(&str, &str, usize)],
    status: u16,
) -> (MockServer, Arc<Mutex<HashMap<String, usize>>>) {
    let inner = FakePortal::new(vec![FakeFormula::sample()]);
    let limits: HashMap<String, usize> = drop
        .iter()
        .map(|(m, p, n)| (format!("{m} {BASE_PATH}{p}"), *n))
        .collect();
    let hits = Arc::new(Mutex::new(HashMap::new()));
    let seen = hits.clone();
    let server = MockServer::start(move |req| {
        let key = format!(
            "{} {}",
            req.method,
            req.path.split('?').next().unwrap_or("")
        );
        let n = {
            let mut hits = seen.lock().expect("hits");
            let n = hits.entry(key.clone()).or_insert(0);
            *n += 1;
            *n
        };
        if n <= limits.get(&key).copied().unwrap_or(0) {
            return MockResponse::text(status, "busy");
        }
        inner.respond(req)
    });
    (server, hits)
}

#[test]
fn lookup_returns_the_formula_and_reuses_the_session() {
    let server = portal();
    let (policy, _) = recording_policy();
    let c = client(server.url(), policy);
    let got = c
        .lookup("CC 900.001", "7770001", "fvs9000001", ARTICLE)
        .expect("formula");
    let want = Formula {
        fecha_formula: "2026-08-15 10:20:30".into(),
        molecula: "ACETAMINOFEN/CODEINA".into(),
        articulo_medicar: "WINA SINT".into(),
        estado_despacho: "DESPACHADO".into(),
        fecha_despacho: "2026-08-16 09:00:00".into(),
        cantidad: Decimal::from(3),
        documento: "900001".into(),
        factura: "FVS9000001".into(),
        ssc: "7770001".into(),
    };
    assert_eq!(got, want);
    // A second invoice of the same patient reuses the login and the list.
    c.lookup("900001", "7770001", "FVS9000001", "ACETAMINOFEN 500")
        .expect("second lookup");
    assert_eq!(count(&server, "GET", "/"), 1);
    assert_eq!(count(&server, "POST", "/procesar.php"), 1);
    assert_eq!(count(&server, "POST", "/modalAjax/Control.php"), 1);
    assert_eq!(count(&server, "POST", "/modalAjax/VerFormula.php"), 2);
    for r in server.requests() {
        assert!(!r.body_text().contains(PASSWORD), "password sent in clear");
        assert!(!r.body_text().contains(USERNAME), "user sent in clear");
    }
}

#[test]
fn permanent_errors_are_not_retried_and_not_remembered() {
    let server = portal();
    let (policy, sleeps) = recording_policy();
    let c = client(server.url(), policy);
    let err = c
        .lookup("", "7770001", "FVS9000001", "X")
        .expect_err("missing document");
    assert!(err.is_permanent(), "{err}");
    let err = c
        .lookup("900001", "7770001", "FVS9000002", "ACETAMINOFEN")
        .expect_err("other invoice");
    assert!(err.is_permanent());
    assert!(
        err.to_string()
            .contains("Medicar no relaciona la formula 7770001 con la factura FVS9000002"),
        "{err}"
    );
    let err = c
        .lookup("900001", "7770001", "FVS9000001", "IBUPROFENO")
        .expect_err("no molecule");
    assert!(
        err.is_permanent() && err.to_string().starts_with("medicar: ninguna molecula"),
        "{err}"
    );
    // A permanent error of one invoice must not stop the next.
    lookup(&c).expect("next invoice");
    assert!(sleeps.lock().expect("log").is_empty());
}

#[test]
fn credentials_are_required() {
    let err = FormulaClient::new(MedicarOptions::new("http://x", "", "", "")).err();
    assert!(err.is_some_and(|e| e.is_permanent()));
}

#[test]
fn each_request_is_retried_up_to_four_attempts_with_1_2_4_second_backoffs() {
    for status in [503, 429, 408] {
        for (method, path) in [
            ("GET", "/"),
            ("POST", "/procesar.php"),
            ("POST", "/modalAjax/Control.php"),
            ("POST", "/modalAjax/VerFormula.php"),
        ] {
            let (server, hits) = broken_front(&[(method, path, 3)], status);
            let (policy, sleeps) = recording_policy();
            let got = lookup(&client(server.url(), policy))
                .unwrap_or_else(|e| panic!("{status} {method} {path}: {e}"));
            assert_eq!(got.cantidad, Decimal::from(3));
            let key = format!("{method} {BASE_PATH}{path}");
            assert_eq!(hits.lock().expect("hits")[&key], 4, "{status} {key}");
            assert_eq!(
                *sleeps.lock().expect("log"),
                [
                    Duration::from_secs(1),
                    Duration::from_secs(2),
                    Duration::from_secs(4)
                ]
            );
            // The retried POST carried its body again: the portal could parse it.
            if method == "POST" {
                let bodies: Vec<_> = server
                    .requests()
                    .into_iter()
                    .filter(|r| r.path.ends_with(path) && r.method == "POST")
                    .map(|r| r.body_text())
                    .collect();
                assert!(
                    bodies.iter().all(|b| b == &bodies[0] && !b.is_empty()),
                    "{bodies:?}"
                );
            }
        }
    }
}

#[test]
fn a_portal_that_stays_down_is_unavailable_and_not_permanent() {
    for status in [503, 429] {
        let (server, hits) = broken_front(&[("GET", "/", 100)], status);
        let (policy, sleeps) = recording_policy();
        let err = lookup(&client(server.url(), policy)).expect_err("outage");
        assert!(err.is_unavailable() && !err.is_permanent(), "{err}");
        assert_eq!(hits.lock().expect("hits")[&format!("GET {BASE_PATH}/")], 4);
        assert_eq!(sleeps.lock().expect("log").len(), 3);
        let text = err.to_string();
        assert!(text.contains("GET /") && text.contains("medicar"), "{text}");
        assert_eq!(
            text,
            "medicar: la consulta a Medicar fallo (GET /: HTTP ".to_owned()
                + &status.to_string()
                + ")"
        );
        for secret in [PASSWORD, USERNAME, "csrf", "PHPSESSID", "sess-sint"] {
            assert!(!text.contains(secret), "leaks {secret}: {text}");
        }
    }
}

#[test]
fn a_transport_failure_is_unavailable_with_only_the_cause() {
    // Nothing listens here: bind a port and close it.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.local_addr().expect("addr").port()
    };
    let (policy, sleeps) = recording_policy();
    let err = lookup(&client(&format!("http://127.0.0.1:{port}"), policy)).expect_err("outage");
    assert!(err.is_unavailable(), "{err}");
    assert_eq!(sleeps.lock().expect("log").len(), 3);
    let text = err.to_string();
    assert!(text.starts_with("medicar: GET /: "), "{text}");
    assert!(!text.contains("http://"), "no URL in {text}");
}

#[test]
fn login_outage_and_rejection() {
    let (server, hits) = broken_front(&[("POST", "/procesar.php", 100)], 502);
    let (policy, _) = recording_policy();
    let err = lookup(&client(server.url(), policy)).expect_err("login outage");
    assert!(err.is_unavailable(), "{err}");
    assert_eq!(
        hits.lock().expect("hits")[&format!("POST {BASE_PATH}/procesar.php")],
        4
    );
    assert_eq!(err.to_string(), "medicar: login: HTTP 502");

    // A 4xx login is permanent and not retried.
    let (server, hits) = broken_front(&[("POST", "/procesar.php", 100)], 403);
    let (policy, sleeps) = recording_policy();
    let err = lookup(&client(server.url(), policy)).expect_err("rejected");
    assert!(err.is_permanent() && !err.is_unavailable(), "{err}");
    assert_eq!(
        err.to_string(),
        "medicar: login was not accepted (HTTP 403)"
    );
    assert_eq!(
        hits.lock().expect("hits")[&format!("POST {BASE_PATH}/procesar.php")],
        1
    );
    assert!(sleeps.lock().expect("log").is_empty());

    // A login the portal answers 200 without "status" is rejected too.
    let server = portal();
    let (policy, _) = recording_policy();
    let mut bad = options(server.url(), policy);
    bad.password = "otra".into();
    let err = lookup(&FormulaClient::new(bad).expect("client")).expect_err("wrong password");
    assert_eq!(
        err.to_string(),
        "medicar: login was not accepted (HTTP 200)"
    );
}

#[test]
fn a_missing_csrf_token_is_permanent() {
    let server = MockServer::start(|_| MockResponse::text(200, "<html>sin formulario</html>"));
    let (policy, _) = recording_policy();
    let err = lookup(&client(server.url(), policy)).expect_err("no csrf");
    assert_eq!(
        err.to_string(),
        "medicar: Medicar no entrego el token de inicio de sesion"
    );
    assert!(err.is_permanent());
}

#[test]
fn a_client_remembers_the_outage_and_stops_sending() {
    let (server, hits) = broken_front(&[("GET", "/", 100)], 503);
    let (policy, sleeps) = recording_policy();
    let c = client(server.url(), policy);
    assert!(lookup(&c).expect_err("first").is_unavailable());
    let (requests, backoffs) = (server.requests().len(), sleeps.lock().expect("log").len());
    for _ in 0..3 {
        let err = lookup(&c).expect_err("after the outage");
        assert!(err.is_unavailable() && !err.is_permanent());
    }
    assert_eq!(
        server.requests().len(),
        requests,
        "no request after the outage"
    );
    assert_eq!(sleeps.lock().expect("log").len(), backoffs);
    assert_eq!(hits.lock().expect("hits")[&format!("GET {BASE_PATH}/")], 4);
}

#[test]
fn clients_can_share_one_outage() {
    let (server, hits) = broken_front(&[("GET", "/", 100)], 503);
    let outage = Arc::new(Outage::new());
    let mk = || {
        let (policy, _) = recording_policy();
        let mut o = options(server.url(), policy);
        o.outage = Some(outage.clone());
        FormulaClient::new(o).expect("client")
    };
    assert!(lookup(&mk()).expect_err("first").is_unavailable());
    assert!(lookup(&mk()).expect_err("second").is_unavailable());
    assert_eq!(
        hits.lock().expect("hits")[&format!("GET {BASE_PATH}/")],
        4,
        "one retry sequence for both clients"
    );
    assert!(outage.err().is_some());
}

#[test]
fn from_settings_names_the_missing_keys() {
    use crate::config::Settings;
    let err = MedicarOptions::from_settings(&Settings::from_pairs([(
        "TESSIA_MEDICAR_BASE_URL",
        "http://x",
    )]))
    .expect_err("incomplete");
    let text = err.to_string();
    assert!(
        text.contains("medicar.identifier") && text.contains("TESSIA_MEDICAR_PASSWORD"),
        "{text}"
    );
    let ok = MedicarOptions::from_settings(&Settings::from_pairs([
        ("TESSIA_MEDICAR_BASE_URL", "http://x/"),
        ("TESSIA_MEDICAR_IDENTIFIER", "i"),
        ("TESSIA_MEDICAR_USERNAME", "u"),
        ("TESSIA_MEDICAR_PASSWORD", "p"),
    ]))
    .expect("complete");
    assert_eq!(ok.base_url, "http://x");
    assert!(
        !format!("{ok:?}").contains("password"),
        "Debug must not print credentials"
    );
}
