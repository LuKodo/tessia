//! The SISPRO client against local mock servers: login and token cache, login
//! retries, the never-retry rule of `CargarFevRips`, the shared limiter, and the
//! TLS options (`minsalud.ca_file`, `minsalud.tls_pin_sha256`) against a local
//! HTTPS server with generated test certificates.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use tessia::http::{AdaptiveLimiter, LimiterOptions};
use tessia::phases::cuv::sispro::{SisproClient, SisproError, SisproOptions};
use tessia::testing::{MockResponse, MockServer};

fn jwt(expires_in: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let engine = &base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = engine.encode(format!(r#"{{"exp":{}}}"#, now + expires_in));
    format!("{}.{payload}.sig", engine.encode(r#"{"alg":"none"}"#))
}

fn options(base: &str) -> SisproOptions {
    SisproOptions {
        base_url: base.to_owned(),
        tipo_doc: "CC".into(),
        numero_doc: "123456".into(),
        clave: "s3cret-clave".into(),
        nit: "900123456".into(),
        ca_file: None,
        tls_pin_sha256: None,
        timeout: Duration::from_secs(5),
        login_attempts: 3,
        login_backoff: Duration::from_millis(1),
        sleep: None,
    }
}

fn client(server: &MockServer) -> SisproClient {
    SisproClient::new(options(server.url()), None).expect("client")
}

fn login_ok(expires_in: i64) -> MockResponse {
    MockResponse::json(200, &format!(r#"{{"token":"{}"}}"#, jwt(expires_in)))
}

/// Routes by path; `cargar` answers CargarFevRips.
fn sispro_server(
    expires_in: i64,
    cargar: impl Fn() -> MockResponse + Send + 'static,
) -> MockServer {
    MockServer::start(move |req| match req.path.as_str() {
        "/api/Auth/LoginSISPRO" => login_ok(expires_in),
        "/api/PaquetesFevRips/CargarFevRips" => cargar(),
        "/api/ConsultasFevRips/ConsultarCUV" => MockResponse::json(200, r#"{"ProcesoId":4242}"#),
        _ => MockResponse::text(404, ""),
    })
}

fn count(server: &MockServer, path: &str) -> usize {
    server.requests().iter().filter(|r| r.path == path).count()
}

const LOGIN: &str = "/api/Auth/LoginSISPRO";
const CARGAR: &str = "/api/PaquetesFevRips/CargarFevRips";
const CONSULTA: &str = "/api/ConsultasFevRips/ConsultarCUV";

#[test]
fn one_login_serves_every_call_with_a_bearer_token() {
    let token = jwt(3600);
    let issued = token.clone();
    let server = MockServer::start(move |req| match req.path.as_str() {
        LOGIN => MockResponse::json(200, &format!(r#"{{"token":"{issued}"}}"#)),
        CARGAR => MockResponse::json(200, r#"{"ok":1}"#),
        _ => MockResponse::json(200, r#"{"ProcesoId":4242}"#),
    });
    let sispro = client(&server);
    for _ in 0..3 {
        let (status, body) = sispro
            .cargar_fev_rips(b"{\"rips\":{}}".to_vec())
            .expect("cargar");
        assert_eq!((status, body.as_slice()), (200, &br#"{"ok":1}"#[..]));
    }
    assert_eq!(sispro.proceso_id("abc"), Some(4242));
    assert_eq!(count(&server, LOGIN), 1);
    let requests = server.requests();
    let login: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("login body");
    assert_eq!(
        login,
        serde_json::json!({
            "persona": {"identificacion": {"tipo": "CC", "numero": "123456"}},
            "clave": "s3cret-clave",
            "nit": "900123456"
        })
    );
    assert_eq!(requests[0].header("authorization"), None);
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    let cargar = requests.iter().find(|r| r.path == CARGAR).expect("cargar");
    assert_eq!(
        cargar.header("authorization"),
        Some(format!("Bearer {token}").as_str())
    );
    assert_eq!(cargar.body, b"{\"rips\":{}}");
    let consulta = requests
        .iter()
        .find(|r| r.path == CONSULTA)
        .expect("consulta");
    assert_eq!(
        consulta.header("content-type"),
        Some("application/json; charset=UTF-8")
    );
    assert_eq!(consulta.header("accept"), Some("application/json"));
    assert_eq!(consulta.body_text(), r#"{"codigoUnicoValidacion":"abc"}"#);
}

#[test]
fn a_token_within_a_minute_of_its_expiry_is_not_reused() {
    let server = sispro_server(30, || MockResponse::json(200, "{}"));
    let sispro = client(&server);
    sispro.cargar_fev_rips(b"{}".to_vec()).expect("first");
    sispro.cargar_fev_rips(b"{}".to_vec()).expect("second");
    assert_eq!(
        count(&server, LOGIN),
        2,
        "cached until one minute before exp"
    );
    let long = sispro_server(3600, || MockResponse::json(200, "{}"));
    let sispro = client(&long);
    sispro.cargar_fev_rips(b"{}".to_vec()).expect("first");
    sispro.cargar_fev_rips(b"{}".to_vec()).expect("second");
    assert_eq!(count(&long, LOGIN), 1);
}

#[test]
fn the_login_is_retried_with_doubling_jittered_backoff() {
    let server = MockServer::sequence(vec![
        MockResponse::text(500, "boom"),
        MockResponse::text(429, "slow down"),
        login_ok(3600),
    ]);
    let sleeps = Arc::new(Mutex::new(Vec::new()));
    let recorder = sleeps.clone();
    let mut opts = options(server.url());
    opts.login_backoff = Duration::from_millis(400);
    opts.sleep = Some(Arc::new(move |d| {
        if let Ok(mut log) = recorder.lock() {
            log.push(d);
        }
    }));
    let sispro = SisproClient::new(opts, None).expect("client");
    sispro
        .cargar_fev_rips(b"{}".to_vec())
        .expect("cargar after retries");
    let sleeps = sleeps.lock().expect("sleeps").clone();
    assert_eq!(sleeps.len(), 2);
    let ms: Vec<f64> = sleeps.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    assert!((300.0..=500.0).contains(&ms[0]), "first wait {}", ms[0]);
    assert!((600.0..=1000.0).contains(&ms[1]), "second wait {}", ms[1]);
    assert_eq!(count(&server, LOGIN), 3);
    assert_eq!(count(&server, CARGAR), 1);
}

#[test]
fn a_200_without_token_is_a_transient_login_failure() {
    let server = MockServer::sequence(vec![MockResponse::json(200, "{}"), login_ok(3600)]);
    let sispro = client(&server);
    assert_eq!(sispro.cargar_fev_rips(b"{}".to_vec()).expect("ok").0, 200);
    assert_eq!(count(&server, LOGIN), 2);
}

#[test]
fn after_three_failed_logins_nothing_is_sent() {
    let server = MockServer::sequence(vec![MockResponse::text(503, "down")]);
    let sispro = client(&server);
    let err = sispro
        .cargar_fev_rips(b"{}".to_vec())
        .expect_err("token error");
    assert!(err.is_token());
    let text = err.to_string();
    assert!(text.starts_with("sispro: token vacio: "), "{text}");
    assert!(text.contains("login fallo tras 3 intentos"), "{text}");
    assert!(
        text.contains("login HTTP 503 sin token (cuerpo: down)"),
        "{text}"
    );
    assert_eq!(count(&server, LOGIN), 3);
    assert_eq!(count(&server, CARGAR), 0);
}

#[test]
fn a_credential_rejection_is_not_retried_and_never_echoes_secrets() {
    let body = r#"{"mensaje":"bad s3cret-clave","detalle":"authorization: Bearer eyJhbGciOi.abc.def","token":null,"clave":"another"}"#;
    let server = MockServer::sequence(vec![MockResponse::json(401, body)]);
    let sispro = client(&server);
    let err = sispro
        .cargar_fev_rips(b"{}".to_vec())
        .expect_err("rejected");
    let text = err.to_string();
    assert_eq!(count(&server, LOGIN), 1, "a 401 is not transient");
    assert!(
        text.contains("login HTTP 401 sin token (cuerpo: "),
        "{text}"
    );
    for secret in ["s3cret-clave", "eyJhbGciOi", "another"] {
        assert!(!text.contains(secret), "{secret} leaked in {text}");
    }
    assert!(text.contains("[redacted]"), "{text}");
}

#[test]
fn a_malformed_token_fails_at_once() {
    let server = MockServer::sequence(vec![MockResponse::json(200, r#"{"token":"not-a-jwt"}"#)]);
    let err = client(&server)
        .cargar_fev_rips(b"{}".to_vec())
        .expect_err("bad jwt");
    assert!(err.to_string().contains("sispro: token: "), "{err}");
    assert_eq!(count(&server, LOGIN), 1);
}

#[test]
fn cargar_fev_rips_is_never_retried() {
    for status in [500, 502, 429, 503] {
        let server = sispro_server(3600, move || MockResponse::text(status, "no"));
        let sispro = client(&server);
        let (got, body) = sispro.cargar_fev_rips(b"{}".to_vec()).expect("answer");
        assert_eq!((got, body.as_slice()), (status, &b"no"[..]));
        assert_eq!(
            count(&server, CARGAR),
            1,
            "status {status} must not be retried"
        );
    }
}

#[test]
fn a_cargar_timeout_is_a_transport_error_sent_once() {
    let server = sispro_server(3600, || {
        std::thread::sleep(Duration::from_millis(600));
        MockResponse::text(200, "late")
    });
    let mut opts = options(server.url());
    opts.timeout = Duration::from_millis(200);
    let sispro = SisproClient::new(opts, None).expect("client");
    let err = sispro.cargar_fev_rips(b"{}".to_vec()).expect_err("timeout");
    assert!(matches!(err, SisproError::Transport(_)), "{err:?}");
    assert!(!err.is_token());
    assert!(
        err.to_string().starts_with("sispro: POST 127.0.0.1:"),
        "{err}"
    );
    std::thread::sleep(Duration::from_millis(900));
    assert_eq!(count(&server, CARGAR), 1);
}

#[test]
fn the_request_timeout_starts_after_the_limiter_slot_is_granted() {
    let server = sispro_server(3600, || MockResponse::json(200, "{}"));
    let limiter = Arc::new(AdaptiveLimiter::new(LimiterOptions {
        name: "test".into(),
        min: 1,
        initial: 1,
        max: 1,
        window: 80,
        pause_min: Duration::ZERO,
        pause_max: Duration::ZERO,
    }));
    let mut opts = options(server.url());
    opts.timeout = Duration::from_millis(300);
    let sispro = SisproClient::new(opts, Some(limiter.clone())).expect("client");
    let busy = limiter.clone();
    let holder = std::thread::spawn(move || {
        let permit = busy.acquire();
        std::thread::sleep(Duration::from_millis(700));
        permit.finish(true);
    });
    std::thread::sleep(Duration::from_millis(100));
    // Waits ~600 ms for the slot (twice the timeout) and still succeeds.
    let (status, _) = sispro
        .cargar_fev_rips(b"{}".to_vec())
        .expect("waited for the slot");
    assert_eq!(status, 200);
    holder.join().expect("holder");
    assert_eq!(limiter.active(), 0, "every slot is given back");
}

#[test]
fn proceso_id_is_none_on_any_failure() {
    let server = MockServer::start(|req| match req.path.as_str() {
        LOGIN => login_ok(3600),
        _ => match req.body_text().as_str() {
            r#"{"codigoUnicoValidacion":"text"}"# => {
                MockResponse::json(200, r#"{"ProcesoId":"77"}"#)
            }
            r#"{"codigoUnicoValidacion":"down"}"# => MockResponse::json(500, r#"{"ProcesoId":1}"#),
            r#"{"codigoUnicoValidacion":"html"}"# => MockResponse::text(200, "<html>"),
            _ => MockResponse::json(200, r#"{"ProcesoId":9}"#),
        },
    });
    let sispro = client(&server);
    assert_eq!(sispro.proceso_id("text"), Some(77));
    assert_eq!(sispro.proceso_id("down"), None);
    assert_eq!(sispro.proceso_id("html"), None);
    assert_eq!(sispro.proceso_id("ok"), Some(9));
    let no_token = MockServer::sequence(vec![MockResponse::text(401, "no")]);
    assert_eq!(client(&no_token).proceso_id("x"), None);
}

#[test]
fn the_base_url_must_be_scheme_host_port() {
    for bad in ["host:9443", "https://h/api", "ftp://h", ""] {
        assert!(SisproClient::new(options(bad), None).is_err(), "{bad}");
    }
}

// ---------------------------------------------------------------------------
// TLS
// ---------------------------------------------------------------------------

fn tls_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cuv/tls")
        .join(name)
}

fn pin_of(cert: &str) -> String {
    let pem = fs::read(tls_fixture(cert)).expect("cert");
    let der = CertificateDer::from_pem_slice(&pem).expect("pem");
    hex::encode(Sha256::digest(der.as_ref()))
}

/// A minimal HTTPS server: login, CargarFevRips, ConsultarCUV. Returns its URL
/// and the number of HTTP requests it answered.
fn tls_server(cert: &str, key: &str) -> (String, Arc<AtomicUsize>) {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(&fs::read(tls_fixture(cert)).expect("cert"))
            .collect::<Result<_, _>>()
            .expect("certs");
    let key =
        PrivateKeyDer::from_pem_slice(&fs::read(tls_fixture(key)).expect("key")).expect("key");
    let config = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .expect("server config"),
    );
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!(
        "https://127.0.0.1:{}",
        listener.local_addr().expect("addr").port()
    );
    let answered = Arc::new(AtomicUsize::new(0));
    let counter = answered.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (config, counter) = (config.clone(), counter.clone());
            std::thread::spawn(move || serve_tls(stream, config, &counter));
        }
    });
    (url, answered)
}

fn serve_tls(mut tcp: TcpStream, config: Arc<rustls::ServerConfig>, answered: &AtomicUsize) {
    let Ok(mut conn) = rustls::ServerConnection::new(config) else {
        return;
    };
    let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        match tls.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
        }
        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
    let wanted: usize = head
        .lines()
        .find_map(|l| {
            l.to_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse().unwrap_or(0))
        })
        .unwrap_or(0);
    while raw.len() < head_end + wanted {
        match tls.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
        }
    }
    let path = head.split_whitespace().nth(1).unwrap_or("");
    let body = match path {
        LOGIN => format!(r#"{{"token":"{}"}}"#, jwt(3600)),
        CARGAR => r#"{"ResultState":true}"#.to_owned(),
        _ => r#"{"ProcesoId":1}"#.to_owned(),
    };
    answered.fetch_add(1, Ordering::SeqCst);
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = tls.write_all(response.as_bytes());
    let _ = tls.flush();
    tls.conn.send_close_notify();
    let _ = tls.flush();
}

fn tls_options(url: &str) -> SisproOptions {
    let mut opts = options(url);
    opts.sleep = Some(Arc::new(|_| {}));
    opts
}

#[test]
fn a_pinned_certificate_without_san_is_trusted() {
    // The real local SISPRO server: CN=localhost, nothing for the host being called.
    let (url, answered) = tls_server("nosan.crt", "nosan.key");
    let mut opts = tls_options(&url);
    opts.tls_pin_sha256 = Some(pin_of("nosan.crt"));
    let sispro = SisproClient::new(opts, None).expect("client");
    let (status, body) = sispro.cargar_fev_rips(b"{}".to_vec()).expect("https call");
    assert_eq!(
        (status, body.as_slice()),
        (200, &br#"{"ResultState":true}"#[..])
    );
    assert_eq!(answered.load(Ordering::SeqCst), 2, "login + CargarFevRips");
}

#[test]
fn the_pin_accepts_the_openssl_fingerprint_spelling() {
    let (url, _) = tls_server("nosan.crt", "nosan.key");
    let plain = pin_of("nosan.crt");
    let colons: Vec<String> = plain
        .as_bytes()
        .chunks(2)
        .map(|c| String::from_utf8_lossy(c).to_uppercase())
        .collect();
    let mut opts = tls_options(&url);
    opts.tls_pin_sha256 = Some(colons.join(":"));
    let sispro = SisproClient::new(opts, None).expect("client");
    assert_eq!(sispro.cargar_fev_rips(b"{}".to_vec()).expect("call").0, 200);
}

#[test]
fn another_certificate_does_not_match_the_pin() {
    let (url, answered) = tls_server("leaf.crt", "leaf.key");
    let mut opts = tls_options(&url);
    opts.tls_pin_sha256 = Some(pin_of("nosan.crt"));
    let sispro = SisproClient::new(opts, None).expect("client");
    let err = sispro
        .cargar_fev_rips(b"{}".to_vec())
        .expect_err("pin mismatch");
    assert!(err.is_token(), "the login could not even start: {err}");
    assert!(
        err.to_string().contains("login fallo tras 3 intentos"),
        "{err}"
    );
    assert_eq!(
        answered.load(Ordering::SeqCst),
        0,
        "no HTTP request got through"
    );
}

#[test]
fn a_wrong_pin_is_rejected_even_for_the_right_server() {
    let (url, answered) = tls_server("nosan.crt", "nosan.key");
    let mut opts = tls_options(&url);
    opts.tls_pin_sha256 = Some("00".repeat(32));
    let sispro = SisproClient::new(opts, None).expect("client");
    assert!(sispro.cargar_fev_rips(b"{}".to_vec()).is_err());
    assert_eq!(answered.load(Ordering::SeqCst), 0);
}

#[test]
fn a_private_ca_file_is_trusted_and_system_roots_are_not() {
    let (url, answered) = tls_server("leaf.crt", "leaf.key");
    let mut with_ca = tls_options(&url);
    with_ca.ca_file = Some(tls_fixture("ca.crt").to_string_lossy().into_owned());
    let sispro = SisproClient::new(with_ca, None).expect("client");
    assert_eq!(
        sispro.cargar_fev_rips(b"{}".to_vec()).expect("trusted").0,
        200
    );
    assert_eq!(answered.load(Ordering::SeqCst), 2);

    let before = answered.load(Ordering::SeqCst);
    let untrusting = SisproClient::new(tls_options(&url), None).expect("client");
    assert!(
        untrusting.cargar_fev_rips(b"{}".to_vec()).is_err(),
        "TLS is verified by default"
    );
    assert_eq!(answered.load(Ordering::SeqCst), before);
}

#[test]
fn a_ca_file_and_a_pin_cannot_be_combined() {
    let mut opts = options("https://127.0.0.1:1");
    opts.ca_file = Some(tls_fixture("ca.crt").to_string_lossy().into_owned());
    opts.tls_pin_sha256 = Some(pin_of("nosan.crt"));
    let err = SisproClient::new(opts, None).expect_err("exclusive");
    assert!(err.to_string().contains("excluyentes"), "{err}");
    let mut bad_pin = options("https://127.0.0.1:1");
    bad_pin.tls_pin_sha256 = Some("xyz".into());
    assert!(SisproClient::new(bad_pin, None).is_err());
}
