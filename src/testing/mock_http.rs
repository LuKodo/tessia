//! Scripted local HTTP server for tests.
//!
//! ```
//! use tessia::testing::{MockResponse, MockServer};
//!
//! let server = MockServer::start(|req| match req.path.as_str() {
//!     "/ping" => MockResponse::text(200, "pong"),
//!     _ => MockResponse::text(404, "nope"),
//! });
//! let body = reqwest::blocking::get(format!("{}/ping", server.url())).unwrap().text().unwrap();
//! assert_eq!(body, "pong");
//! assert_eq!(server.requests().len(), 1);
//! ```

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// A request the server received.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    /// Path with the query string, e.g. `/api/v1/files?limit=2`.
    pub path: String,
    /// Header names are lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    /// First header value with that (case-insensitive) name.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// What the server answers.
#[derive(Debug, Clone)]
pub struct MockResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl MockResponse {
    pub fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".into(), "text/plain".into())],
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".into(), "application/octet-stream".into())],
            body,
        }
    }

    #[must_use]
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// A local server on a random port; stops when dropped.
pub struct MockServer {
    server: Arc<tiny_http::Server>,
    url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    worker: Option<JoinHandle<()>>,
}

impl MockServer {
    /// Starts the server; `handler` answers every request.
    pub fn start(
        handler: impl Fn(&RecordedRequest) -> MockResponse + Send + 'static,
    ) -> MockServer {
        let server = match tiny_http::Server::http("127.0.0.1:0") {
            Ok(server) => Arc::new(server),
            Err(err) => panic!("no se pudo iniciar el servidor de prueba: {err}"),
        };
        let url = format!("http://{}", server.server_addr());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let worker = {
            let (server, requests) = (server.clone(), requests.clone());
            std::thread::spawn(move || {
                for mut request in server.incoming_requests() {
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                    let recorded = RecordedRequest {
                        method: request.method().to_string(),
                        path: request.url().to_owned(),
                        headers: request
                            .headers()
                            .iter()
                            .map(|h| (h.field.to_string().to_lowercase(), h.value.to_string()))
                            .collect(),
                        body,
                    };
                    let response = handler(&recorded);
                    if let Ok(mut log) = requests.lock() {
                        log.push(recorded);
                    }
                    let mut out = tiny_http::Response::from_data(response.body)
                        .with_status_code(response.status);
                    for (name, value) in &response.headers {
                        if let Ok(header) =
                            tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes())
                        {
                            out.add_header(header);
                        }
                    }
                    let _ = request.respond(out);
                }
            })
        };
        MockServer {
            server,
            url,
            requests,
            worker: Some(worker),
        }
    }

    /// Answers `responses` in order (the last one repeats forever).
    pub fn sequence(responses: Vec<MockResponse>) -> MockServer {
        let position = Mutex::new(0usize);
        MockServer::start(move |_| {
            let mut n = position.lock().map(|g| *g).unwrap_or(0);
            let last = responses.len().saturating_sub(1);
            let response = responses[n.min(last)].clone();
            n += 1;
            if let Ok(mut guard) = position.lock() {
                *guard = n;
            }
            response
        })
    }

    /// Base URL, e.g. `http://127.0.0.1:41233` (no trailing slash).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Requests received so far, in order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
