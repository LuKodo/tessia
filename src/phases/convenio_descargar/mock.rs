//! `MockMedicar`: a local fake of the Medicar portal for tests (port of Go
//! `medicartest`, extended to the convenio export). Synthetic data only.
//!
//! It serves, under [`BASE_PATH`] and [`PRODUCTS_PATH`], the login page (csrf
//! token), `procesar.php`, the report page, the export (which answers with a
//! CSV link) and the CSV itself. Every page except the login page needs the
//! session cookie handed out by `GET {base}/`, and every request needs the
//! browser User-Agent (the real portal answers 403 to others).

use std::sync::{Arc, Mutex};

use crate::phases::convenio_descargar::client::{MedicarOptions, md5_hex};
use crate::testing::{MockResponse, MockServer, RecordedRequest};

/// Portal path of the login site (`medicar.base_url` = server URL + this).
pub const BASE_PATH: &str = "/pharmaser/mutualser";
/// Portal path of the reports (`medicar.products_url` = server URL + this).
pub const PRODUCTS_PATH: &str = "/pharmaser/productosDespachados";
/// Credentials the fake accepts.
pub const IDENTIFIER: &str = "ID-SINT";
pub const USERNAME: &str = "usuario-sint";
pub const PASSWORD: &str = "clave-sint";
/// Token served in the login page (entity-encoded there).
pub const CSRF_TOKEN: &str = "tok&sint";
/// Path of the CSV, relative to [`PRODUCTS_PATH`].
pub const CSV_PATH: &str = "descargas/Convenio.csv";

/// What the login does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Login {
    Accepted,
    /// `procesar.php` answers 200 without `"status"`.
    Rejected,
    /// The login page has no csrf token.
    NoCsrf,
}

/// What the export answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// `parent.location='http://host/.../Convenio.csv'`
    Absolute,
    /// `parent.location='descargas/Convenio.csv'`
    Relative,
    /// A page without any CSV link ("no data for the range").
    Missing,
}

struct Failure {
    suffix: String,
    status: u16,
    remaining: u32,
}

struct State {
    login: Login,
    link: Link,
    csv: Vec<u8>,
    failures: Vec<Failure>,
    sessions: u32,
}

/// The fake portal. Dropping it stops the server.
pub struct MockMedicar {
    server: MockServer,
    state: Arc<Mutex<State>>,
}

impl MockMedicar {
    /// Starts the portal on an ephemeral port: login accepted, absolute link,
    /// a small CSV.
    pub fn start() -> MockMedicar {
        let state = Arc::new(Mutex::new(State {
            login: Login::Accepted,
            link: Link::Absolute,
            csv: b"Factura|Plu\nFEV1|7\n".to_vec(),
            failures: Vec::new(),
            sessions: 0,
        }));
        let shared = state.clone();
        let server = MockServer::start(move |request| match shared.lock() {
            Ok(mut state) => respond(&mut state, request),
            Err(_) => MockResponse::text(500, "mock poisoned"),
        });
        MockMedicar { server, state }
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut State) -> T) -> Option<T> {
        self.state.lock().ok().map(|mut state| f(&mut state))
    }

    pub fn set_login(&self, login: Login) {
        self.with_state(|s| s.login = login);
    }

    pub fn set_link(&self, link: Link) {
        self.with_state(|s| s.link = link);
    }

    pub fn set_csv(&self, csv: &[u8]) {
        self.with_state(|s| s.csv = csv.to_vec());
    }

    /// Answers `status` to the next `times` requests whose path ends with
    /// `path_suffix` (any method).
    pub fn fail(&self, path_suffix: &str, status: u16, times: u32) {
        self.with_state(|s| {
            s.failures.push(Failure {
                suffix: path_suffix.to_owned(),
                status,
                remaining: times,
            });
        });
    }

    /// `medicar.base_url` of this portal.
    pub fn base_url(&self) -> String {
        format!("{}{BASE_PATH}", self.server.url())
    }

    /// `medicar.products_url` of this portal.
    pub fn products_url(&self) -> String {
        format!("{}{PRODUCTS_PATH}", self.server.url())
    }

    /// Client options pointing at this portal, with no waiting between retries.
    pub fn options(&self) -> MedicarOptions {
        MedicarOptions::new(
            &self.base_url(),
            &self.products_url(),
            IDENTIFIER,
            USERNAME,
            PASSWORD,
        )
        .map(MedicarOptions::with_fast_retries)
        .unwrap_or_else(|err| panic!("opciones del portal falso: {err}"))
    }

    /// Every request received, in arrival order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.server.requests()
    }

    /// Requests with this method whose path (without query) ends with `suffix`.
    pub fn count(&self, method: &str, suffix: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.method == method && path_of(r).ends_with(suffix))
            .count()
    }
}

fn path_of(request: &RecordedRequest) -> &str {
    request.path.split('?').next().unwrap_or_default()
}

fn form_value(body: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(body.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn respond(state: &mut State, request: &RecordedRequest) -> MockResponse {
    let path = path_of(request).to_owned();
    if !request
        .header("user-agent")
        .is_some_and(|ua| ua.contains("Chrome/128"))
    {
        return MockResponse::text(403, "forbidden");
    }
    if let Some(failure) = state
        .failures
        .iter_mut()
        .find(|f| f.remaining > 0 && path.ends_with(&f.suffix))
    {
        failure.remaining -= 1;
        return MockResponse::text(failure.status, "scripted failure");
    }
    let login_page = format!("{BASE_PATH}/");
    if request.method == "GET" && path == login_page {
        state.sessions += 1;
        let html = if state.login == Login::NoCsrf {
            "<form></form>".to_owned()
        } else {
            // value before name: the order the client also accepts.
            r#"<form><input type="hidden" value="tok&amp;sint" name="csrf_token"></form>"#
                .to_owned()
        };
        return MockResponse::text(200, &html).with_header(
            "Set-Cookie",
            &format!("PHPSESSID=sess-{}; Path=/", state.sessions),
        );
    }
    let has_session = request
        .header("cookie")
        .is_some_and(|c| c.contains("PHPSESSID=sess-"));
    if !has_session {
        return MockResponse::text(403, "no session");
    }
    match (request.method.as_str(), path.as_str()) {
        ("POST", p) if p == format!("{BASE_PATH}/procesar.php") => login(state, request),
        ("GET", p) if p == format!("{PRODUCTS_PATH}/convenioArticulos.php") => {
            MockResponse::text(200, "<html>informe de convenio</html>")
        }
        ("POST", p) if p == format!("{PRODUCTS_PATH}/exportarExcelConvenioArticulos.php") => {
            export(state, request)
        }
        ("GET", p) if p == format!("{PRODUCTS_PATH}/{CSV_PATH}") => {
            MockResponse::bytes(200, state.csv.clone())
        }
        _ => MockResponse::text(404, "not found"),
    }
}

fn login(state: &State, request: &RecordedRequest) -> MockResponse {
    let body = request.body_text();
    let valid = state.login == Login::Accepted
        && form_value(&body, "csrf_token").as_deref() == Some(CSRF_TOKEN)
        && form_value(&body, "f_id").as_deref() == Some(IDENTIFIER)
        && form_value(&body, "f_log").as_deref() == Some(md5_hex(USERNAME).as_str())
        && form_value(&body, "f_pass").as_deref() == Some(md5_hex(PASSWORD).as_str());
    if valid {
        MockResponse::json(200, r#"{"status":1}"#)
    } else {
        MockResponse::json(200, r#"{"error":"login"}"#)
    }
}

fn export(state: &State, request: &RecordedRequest) -> MockResponse {
    let host = request.header("host").unwrap_or("127.0.0.1");
    let page = match state.link {
        Link::Absolute => {
            format!("<script>parent.location='http://{host}{PRODUCTS_PATH}/{CSV_PATH}';</script>")
        }
        Link::Relative => format!("<script>parent.location='{CSV_PATH}';</script>"),
        Link::Missing => "<html>Sin datos para el rango</html>".to_owned(),
    };
    MockResponse::text(200, &page)
}
