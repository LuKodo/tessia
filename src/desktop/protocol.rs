//! The custom-protocol bridge: what the webview asks for goes either to the
//! embedded UI (static rules in [`assets`](super::assets)) or, for `/api` and
//! `/api/*`, in-process to [`ApiState::handle`](crate::api::ApiState::handle).
//! Nothing listens on a socket.
//!
//! # Threads
//!
//! wry calls the protocol handler on the UI thread, so blocking Diesel or HTTP
//! work must never run there. API requests are queued to a small worker pool
//! and answered through the asynchronous responder; static files are answered
//! inline (a memory copy) so a page load never waits behind a slow API call.
//!
//! # App origin
//!
//! The page origin is `tessia://localhost` on macOS and Linux and
//! `http://tessia.localhost` on Windows (WebView2 only serves custom schemes
//! through that http mapping; wry reverts the URI before calling the handler, so
//! the handler sees `tessia://localhost/...` on every OS but the `Origin` a
//! `fetch` sends keeps the page's form). Browsers send `Origin` on non-GET
//! requests and `Sec-Fetch-Site` where the engine supports it; both are forwarded
//! untouched so the API's same-origin guard can judge them. Like Go's desktop
//! handler (`desktopAPIRequest`) the bridge also vouches for the window's own
//! origin: when `Origin` equals the app origin it adds [`TRUSTED_ORIGIN_HEADER`]
//! (and strips any `x-tessia-*` header the page sent, so a page cannot forge it).

use std::borrow::Cow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use http::header::{ALLOW, CACHE_CONTROL, CONTENT_TYPE, HeaderName, HeaderValue};
use http::{Method, StatusCode};
use percent_encoding::percent_decode_str;

use super::assets::{self, AssetSource};
use crate::api::{Request, Response};

/// The custom URI scheme the window loads.
pub const SCHEME: &str = "tessia";

/// Header the bridge adds (value `1`) when the request's `Origin` is the app's
/// own origin. Every inbound header with the `x-tessia-` prefix is dropped first.
pub const TRUSTED_ORIGIN_HEADER: &str = "x-tessia-app-origin";

/// Request headers dropped before the API sees them: the answer must always be a
/// plain 200 (a 304 becomes a 500 in some webviews) and bodies are handed over
/// uncompressed.
const STRIPPED_HEADERS: [&str; 3] = ["if-none-match", "if-modified-since", "accept-encoding"];

/// Workers that run API requests.
pub const API_WORKERS: usize = 8;

/// `Origin` of the page on this OS.
pub fn app_origin() -> &'static str {
    origin_for(cfg!(windows))
}

/// `Origin` of the page on Windows (`windows`) or everywhere else.
pub fn origin_for(windows: bool) -> &'static str {
    if windows {
        "http://tessia.localhost"
    } else {
        "tessia://localhost"
    }
}

/// The URL the window opens.
pub fn start_url() -> String {
    format!("{}/", app_origin())
}

/// True when `url` is the app origin itself or a path, query or fragment on it
/// (case-insensitive on the scheme and host). Used to lock navigation.
pub fn is_app_url(url: &str, origin: &str) -> bool {
    let Some(head) = url.get(..origin.len()) else {
        return false;
    };
    head.eq_ignore_ascii_case(origin)
        && matches!(
            url[origin.len()..].chars().next(),
            None | Some('/' | '?' | '#')
        )
}

/// `/api` and `/api/*` go to the API; everything else is the static UI.
pub fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

/// Runs one API request. In production this is `ApiState::handle`.
pub type ApiFn = dyn Fn(Request) -> Response + Send + Sync;

/// Delivers the answer to the webview (wry's `RequestAsyncResponder::respond`).
pub type Respond = Box<dyn FnOnce(http::Response<Cow<'static, [u8]>>) + Send>;

/// Routes protocol requests to the static UI or the API worker pool.
pub struct Bridge {
    assets: Arc<dyn AssetSource>,
    api: Arc<ApiFn>,
    origin: &'static str,
    pool: WorkerPool,
    closing: Arc<AtomicBool>,
}

impl Bridge {
    pub fn new(assets: Arc<dyn AssetSource>, api: Arc<ApiFn>, origin: &'static str) -> Self {
        Self::with_workers(assets, api, origin, API_WORKERS)
    }

    pub fn with_workers(
        assets: Arc<dyn AssetSource>,
        api: Arc<ApiFn>,
        origin: &'static str,
        workers: usize,
    ) -> Self {
        Self {
            assets,
            api,
            origin,
            pool: WorkerPool::new(workers),
            closing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Answers one request through `respond`, possibly from another thread.
    pub fn handle(&self, request: http::Request<Vec<u8>>, respond: Respond) {
        let path = match decode_path(request.uri().path()) {
            Some(path) => path,
            None => return respond(text_response(StatusCode::BAD_REQUEST, "bad request\n")),
        };
        if !is_api_path(&path) {
            return respond(static_response(
                self.assets.as_ref(),
                request.method(),
                &path,
            ));
        }
        let api_request = to_api_request(request, path, self.origin);
        let api = Arc::clone(&self.api);
        let closing = Arc::clone(&self.closing);
        self.pool.execute(Box::new(move || {
            let answer = run_api(api.as_ref(), api_request);
            // Once the window is closing nobody can receive the answer.
            if !closing.load(Ordering::SeqCst) {
                respond(answer);
            }
        }));
    }

    /// Stops accepting API work, drops what is queued and waits up to `wait` for
    /// requests already running (a request stuck on the network is left behind;
    /// the process is about to exit).
    pub fn close(&self, wait: Duration) {
        self.closing.store(true, Ordering::SeqCst);
        self.pool.shutdown(wait);
    }
}

fn run_api(api: &ApiFn, request: Request) -> http::Response<Cow<'static, [u8]>> {
    match catch_unwind(AssertUnwindSafe(|| api(request))) {
        Ok(response) => api_response(response),
        Err(_) => {
            tracing::error!("an API handler panicked");
            api_response(Response::json_status(
                500,
                &serde_json::json!({ "error": "error interno" }),
            ))
        }
    }
}

/// Percent-decodes a URI path (Go's `URL.Path`); `None` when it is not UTF-8.
fn decode_path(raw: &str) -> Option<String> {
    percent_decode_str(raw)
        .decode_utf8()
        .ok()
        .map(Cow::into_owned)
}

/// Translates a protocol request into the API's framework-free [`Request`].
///
/// Header names are lower-case (first value wins), the conditional and
/// compression headers are dropped, every `x-tessia-*` header is dropped, and
/// [`TRUSTED_ORIGIN_HEADER`] is added when `Origin` is the app origin. Query
/// pairs are decoded in order; the body is moved, never copied.
pub fn to_api_request(request: http::Request<Vec<u8>>, path: String, app_origin: &str) -> Request {
    let (parts, body) = request.into_parts();
    let mut api = Request::new(parts.method.as_str(), &path);
    if let Some(query) = parts.uri.query() {
        api.query = url::form_urlencoded::parse(query.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
    }
    for (name, value) in &parts.headers {
        let name = name.as_str();
        if STRIPPED_HEADERS.contains(&name) || name.starts_with("x-tessia-") {
            continue;
        }
        if let Ok(value) = value.to_str() {
            api.headers
                .entry(name.to_string())
                .or_insert_with(|| value.to_string());
        }
    }
    let own_origin = api
        .headers
        .get("origin")
        .is_some_and(|origin| origin.eq_ignore_ascii_case(app_origin));
    if own_origin {
        api.headers
            .insert(TRUSTED_ORIGIN_HEADER.to_string(), "1".to_string());
    }
    api.body = body;
    api
}

/// Translates the API's [`Response`]. Invalid header names or values are
/// skipped. An answer without `Cache-Control` gets `no-store`, so personal data
/// never lands in the webview's disk cache.
pub fn api_response(response: Response) -> http::Response<Cow<'static, [u8]>> {
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = http::Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        for (name, value) in &response.headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.append(name, value);
            }
        }
        if !headers.contains_key(CACHE_CONTROL) {
            headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        }
        headers.insert(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        );
    }
    finish(builder, Cow::Owned(response.body))
}

/// The static UI's answer (Go: `staticUIHandler`).
pub fn static_response(
    source: &dyn AssetSource,
    method: &Method,
    path: &str,
) -> http::Response<Cow<'static, [u8]>> {
    if method != Method::GET && method != Method::HEAD {
        let mut response = text_response(StatusCode::METHOD_NOT_ALLOWED, "method not allowed\n");
        response
            .headers_mut()
            .insert(ALLOW, HeaderValue::from_static("GET, HEAD"));
        return response;
    }
    let Some(asset) = assets::resolve(source, path) else {
        return text_response(StatusCode::NOT_FOUND, "404 page not found\n");
    };
    let body = if method == Method::HEAD {
        Cow::Borrowed(&[][..])
    } else {
        asset.data
    };
    let mut builder = http::Response::builder()
        .status(StatusCode::OK)
        .header(CACHE_CONTROL, "no-cache")
        .header("x-content-type-options", "nosniff");
    if let Ok(value) = HeaderValue::from_str(&assets::content_type(&asset.name)) {
        builder = builder.header(CONTENT_TYPE, value);
    }
    finish(builder, body)
}

fn text_response(status: StatusCode, message: &'static str) -> http::Response<Cow<'static, [u8]>> {
    let builder = http::Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("x-content-type-options", "nosniff");
    finish(builder, Cow::Borrowed(message.as_bytes()))
}

/// Builds the response; the builder only fails on an invalid status or header
/// built by this module, which a test would have caught.
fn finish(
    builder: http::response::Builder,
    body: Cow<'static, [u8]>,
) -> http::Response<Cow<'static, [u8]>> {
    builder.body(body).unwrap_or_else(|_| {
        let mut fallback = http::Response::new(Cow::Borrowed(&b"internal error\n"[..]));
        *fallback.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        fallback
    })
}

type Job = Box<dyn FnOnce() + Send>;

/// A fixed set of threads fed from one channel.
struct WorkerPool {
    sender: Mutex<Option<Sender<Job>>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl WorkerPool {
    fn new(size: usize) -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded::<Job>();
        let threads = (0..size.max(1))
            .filter_map(|index| spawn_worker(index, receiver.clone()))
            .collect();
        Self {
            sender: Mutex::new(Some(sender)),
            threads: Mutex::new(threads),
        }
    }

    /// Queues `job`; it is dropped when the pool is shut down.
    fn execute(&self, job: Job) {
        let sender = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = sender.as_ref() {
            let _ = sender.send(job);
        }
    }

    /// Closes the queue (queued jobs still run, new ones are dropped) and joins
    /// the workers that finish within `wait`.
    fn shutdown(&self, wait: Duration) {
        self.sender.lock().unwrap_or_else(|e| e.into_inner()).take();
        let mut threads =
            std::mem::take(&mut *self.threads.lock().unwrap_or_else(|e| e.into_inner()));
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline && threads.iter().any(|t| !t.is_finished()) {
            std::thread::sleep(Duration::from_millis(10));
        }
        for thread in threads.drain(..).filter(|t| t.is_finished()) {
            let _ = thread.join();
        }
    }
}

fn spawn_worker(index: usize, receiver: Receiver<Job>) -> Option<JoinHandle<()>> {
    std::thread::Builder::new()
        .name(format!("tessia-api-{index}"))
        .spawn(move || {
            while let Ok(job) = receiver.recv() {
                job();
            }
        })
        .map_err(|error| tracing::error!(%error, "could not start an API worker"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::assets::MemoryAssets;
    use std::sync::mpsc;

    fn req(method: &str, uri: &str) -> http::request::Builder {
        http::Request::builder().method(method).uri(uri)
    }

    fn ui() -> Arc<MemoryAssets> {
        Arc::new(
            MemoryAssets::new()
                .with("index.html", "<html>shell</html>")
                .with("assets/app.js", "export {}"),
        )
    }

    /// An API that echoes what it received as JSON.
    fn echo_api() -> Arc<ApiFn> {
        Arc::new(|request: Request| {
            let mut headers: Vec<_> = request.headers.iter().collect();
            headers.sort();
            Response::json(&serde_json::json!({
                "method": request.method,
                "path": request.path,
                "query": request.query,
                "headers": headers,
                "body": String::from_utf8_lossy(&request.body),
            }))
        })
    }

    fn call(
        bridge: &Bridge,
        request: http::Request<Vec<u8>>,
    ) -> http::Response<Cow<'static, [u8]>> {
        let (tx, rx) = mpsc::channel();
        bridge.handle(
            request,
            Box::new(move |response| {
                let _ = tx.send(response);
            }),
        );
        rx.recv_timeout(Duration::from_secs(5)).expect("no answer")
    }

    fn body_json(response: &http::Response<Cow<'static, [u8]>>) -> serde_json::Value {
        serde_json::from_slice(response.body()).expect("json body")
    }

    #[test]
    fn app_origin_depends_on_the_os() {
        assert_eq!(origin_for(false), "tessia://localhost");
        assert_eq!(origin_for(true), "http://tessia.localhost");
        assert_eq!(start_url(), format!("{}/", app_origin()));
    }

    #[test]
    fn navigation_is_locked_to_the_app_origin() {
        let origin = "tessia://localhost";
        for url in [
            "tessia://localhost",
            "tessia://localhost/",
            "tessia://localhost/ejecuciones?x=1#top",
            "TESSIA://LOCALHOST/api/x",
        ] {
            assert!(is_app_url(url, origin), "{url}");
        }
        for url in [
            "https://example.com/",
            "http://tessia.localhost/",
            "tessia://localhost.evil.com/",
            "tessia://localhostx/",
            "tessia://other/",
            "file:///etc/passwd",
            "about:blank",
            "javascript:alert(1)",
            "",
        ] {
            assert!(!is_app_url(url, origin), "{url}");
        }
        assert!(is_app_url(
            "http://tessia.localhost/a",
            "http://tessia.localhost"
        ));
    }

    #[test]
    fn api_paths_are_api_and_api_only() {
        assert!(is_api_path("/api"));
        assert!(is_api_path("/api/health"));
        assert!(!is_api_path("/apis"));
        assert!(!is_api_path("/apixyz/a"));
        assert!(!is_api_path("/"));
        assert!(!is_api_path("/ejecuciones"));
    }

    #[test]
    fn the_api_request_keeps_the_method_path_query_and_body() {
        let bridge = Bridge::new(ui(), echo_api(), "tessia://localhost");
        let request = req(
            "post",
            "tessia://localhost/api/cargues/Cargue%20A/informe?formato=csv&q=a%20b+c&q=2",
        )
        .header("content-type", "application/json")
        .body(b"{\"Cargue\":\"x\"}".to_vec())
        .unwrap();
        let response = call(&bridge, request);
        assert_eq!(response.status(), 200);
        let json = body_json(&response);
        assert_eq!(json["method"], "POST");
        assert_eq!(json["path"], "/api/cargues/Cargue A/informe");
        assert_eq!(
            json["query"],
            serde_json::json!([["formato", "csv"], ["q", "a b c"], ["q", "2"]])
        );
        assert_eq!(json["body"], "{\"Cargue\":\"x\"}");
        assert!(json["headers"].to_string().contains("application/json"));
    }

    #[test]
    fn conditional_and_compression_headers_never_reach_the_api() {
        let bridge = Bridge::new(ui(), echo_api(), "tessia://localhost");
        let request = req("GET", "tessia://localhost/api/health")
            .header("If-None-Match", "\"abc\"")
            .header("If-Modified-Since", "Wed, 21 Oct 2026 07:28:00 GMT")
            .header("Accept-Encoding", "gzip, br")
            .header("Accept", "application/json")
            .body(Vec::new())
            .unwrap();
        let headers = body_json(&call(&bridge, request))["headers"].to_string();
        for stripped in ["if-none-match", "if-modified-since", "accept-encoding"] {
            assert!(!headers.contains(stripped), "{stripped} leaked: {headers}");
        }
        assert!(headers.contains("accept"));
    }

    #[test]
    fn origin_and_sec_fetch_site_are_forwarded() {
        let bridge = Bridge::new(ui(), echo_api(), "tessia://localhost");
        let request = req("POST", "tessia://localhost/api/run")
            .header("Origin", "https://evil.example")
            .header("Sec-Fetch-Site", "cross-site")
            .body(Vec::new())
            .unwrap();
        let headers = body_json(&call(&bridge, request))["headers"].to_string();
        assert!(headers.contains("https://evil.example"), "{headers}");
        assert!(headers.contains("cross-site"), "{headers}");
        assert!(!headers.contains(TRUSTED_ORIGIN_HEADER), "{headers}");
    }

    #[test]
    fn the_windows_origin_is_trusted_on_windows_only_bridges() {
        let windows = Bridge::new(ui(), echo_api(), origin_for(true));
        let request = req("POST", "tessia://localhost/api/run")
            .header("Origin", "http://tessia.localhost")
            .body(Vec::new())
            .unwrap();
        let headers = body_json(&call(&windows, request))["headers"].to_string();
        assert!(headers.contains(TRUSTED_ORIGIN_HEADER), "{headers}");

        // The same Origin on a bridge for another OS is just a foreign origin.
        let other = Bridge::new(ui(), echo_api(), origin_for(false));
        let request = req("POST", "tessia://localhost/api/run")
            .header("Origin", "http://tessia.localhost")
            .body(Vec::new())
            .unwrap();
        let headers = body_json(&call(&other, request))["headers"].to_string();
        assert!(!headers.contains(TRUSTED_ORIGIN_HEADER), "{headers}");
    }

    #[test]
    fn a_page_cannot_forge_the_trust_marker() {
        let bridge = Bridge::new(ui(), echo_api(), "tessia://localhost");
        let request = req("POST", "tessia://localhost/api/run")
            .header(TRUSTED_ORIGIN_HEADER, "1")
            .header("X-Tessia-Anything", "1")
            .header("Origin", "https://evil.example")
            .body(Vec::new())
            .unwrap();
        let headers = body_json(&call(&bridge, request))["headers"].to_string();
        assert!(!headers.contains("x-tessia"), "{headers}");
    }

    #[test]
    fn the_own_origin_gets_the_marker_case_insensitively() {
        let bridge = Bridge::new(ui(), echo_api(), "tessia://localhost");
        let request = req("PUT", "tessia://localhost/api/parameters")
            .header("Origin", "TESSIA://localhost")
            .body(Vec::new())
            .unwrap();
        let headers = body_json(&call(&bridge, request))["headers"].to_string();
        assert!(headers.contains(TRUSTED_ORIGIN_HEADER), "{headers}");
    }

    #[test]
    fn api_answers_default_to_no_store_and_keep_their_own_headers() {
        let api: Arc<ApiFn> = Arc::new(|request: Request| {
            if request.path == "/api/portal" {
                Response::json(&serde_json::json!({}))
                    .with_header("Cache-Control", "no-store, private")
            } else {
                Response::json_status(409, &serde_json::json!({"error": "x"}))
                    .with_header("X-Custom", "1")
            }
        });
        let bridge = Bridge::new(ui(), api, "tessia://localhost");
        let plain = call(
            &bridge,
            req("GET", "tessia://localhost/api/other")
                .body(Vec::new())
                .unwrap(),
        );
        assert_eq!(plain.status(), 409);
        assert_eq!(plain.headers()["cache-control"], "no-store");
        assert_eq!(plain.headers()["x-custom"], "1");
        assert_eq!(
            plain.headers()["content-type"],
            "application/json; charset=utf-8"
        );
        let own = call(
            &bridge,
            req("GET", "tessia://localhost/api/portal")
                .body(Vec::new())
                .unwrap(),
        );
        assert_eq!(own.headers()["cache-control"], "no-store, private");
        assert_eq!(own.headers().get_all("cache-control").iter().count(), 1);
    }

    #[test]
    fn a_panicking_handler_becomes_a_500_and_the_pool_survives() {
        let api: Arc<ApiFn> = Arc::new(|request: Request| {
            if request.path == "/api/boom" {
                panic!("boom");
            }
            Response::json(&serde_json::json!({"ok": true}))
        });
        let bridge = Bridge::with_workers(ui(), api, "tessia://localhost", 1);
        let boom = call(
            &bridge,
            req("GET", "tessia://localhost/api/boom")
                .body(Vec::new())
                .unwrap(),
        );
        assert_eq!(boom.status(), 500);
        assert_eq!(
            body_json(&boom),
            serde_json::json!({"error": "error interno"})
        );
        let fine = call(
            &bridge,
            req("GET", "tessia://localhost/api/ok")
                .body(Vec::new())
                .unwrap(),
        );
        assert_eq!(fine.status(), 200);
    }

    #[test]
    fn api_requests_run_off_the_calling_thread_and_do_not_block_it() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let api: Arc<ApiFn> = Arc::new(move |_request: Request| {
            let _ = release_rx
                .lock()
                .map(|rx| rx.recv_timeout(Duration::from_secs(5)));
            Response::json(&serde_json::json!({"thread": std::thread::current().name()}))
        });
        let bridge = Bridge::new(ui(), api, "tessia://localhost");
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        bridge.handle(
            req("GET", "tessia://localhost/api/slow")
                .body(Vec::new())
                .unwrap(),
            Box::new(move |response| {
                let _ = tx.send(response);
            }),
        );
        // handle() returned while the API call is still parked.
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(rx.try_recv().is_err());
        // Static files are answered inline even while the API is busy.
        assert_eq!(
            call(
                &bridge,
                req("GET", "tessia://localhost/").body(Vec::new()).unwrap()
            )
            .status(),
            200
        );
        release_tx.send(()).unwrap();
        let answer = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let thread = body_json(&answer)["thread"].as_str().unwrap().to_string();
        assert!(thread.starts_with("tessia-api-"), "{thread}");
    }

    #[test]
    fn a_closed_bridge_stops_answering() {
        let bridge = Bridge::new(ui(), echo_api(), "tessia://localhost");
        bridge.close(Duration::from_millis(200));
        let (tx, rx) = mpsc::channel();
        bridge.handle(
            req("GET", "tessia://localhost/api/health")
                .body(Vec::new())
                .unwrap(),
            Box::new(move |response| {
                let _ = tx.send(response);
            }),
        );
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn static_files_follow_the_serving_rules() {
        let bridge = Bridge::new(ui(), echo_api(), "tessia://localhost");
        let get = |path: &str| {
            call(
                &bridge,
                req("GET", &format!("tessia://localhost{path}"))
                    .body(Vec::new())
                    .unwrap(),
            )
        };

        let index = get("/");
        assert_eq!(index.status(), 200);
        assert_eq!(index.headers()["content-type"], "text/html; charset=utf-8");
        assert_eq!(index.body().as_ref(), b"<html>shell</html>");

        let js = get("/assets/app.js");
        assert_eq!(js.status(), 200);
        assert!(
            js.headers()["content-type"]
                .to_str()
                .unwrap()
                .contains("javascript")
        );

        // SPA fallback for an extension-less client route, 404 for a missing file.
        assert_eq!(get("/ejecuciones").body().as_ref(), b"<html>shell</html>");
        let missing = get("/assets/missing.js");
        assert_eq!(missing.status(), 404);
        assert_eq!(missing.body().as_ref(), b"404 page not found\n");
        // `/apis` is not the API.
        assert_eq!(get("/apis").status(), 200);
    }

    #[test]
    fn head_has_headers_and_no_body() {
        let bridge = Bridge::new(ui(), echo_api(), "tessia://localhost");
        let head = call(
            &bridge,
            req("HEAD", "tessia://localhost/").body(Vec::new()).unwrap(),
        );
        assert_eq!(head.status(), 200);
        assert_eq!(head.headers()["content-type"], "text/html; charset=utf-8");
        assert!(head.body().is_empty());
    }

    #[test]
    fn non_get_head_on_static_is_405_with_allow() {
        let bridge = Bridge::new(ui(), echo_api(), "tessia://localhost");
        for method in ["POST", "PUT", "DELETE", "PATCH"] {
            let response = call(
                &bridge,
                req(method, "tessia://localhost/").body(Vec::new()).unwrap(),
            );
            assert_eq!(response.status(), 405, "{method}");
            assert_eq!(response.headers()["allow"], "GET, HEAD");
            assert_eq!(response.body().as_ref(), b"method not allowed\n");
        }
    }

    #[test]
    fn a_non_utf8_path_is_a_400() {
        let bridge = Bridge::new(ui(), echo_api(), "tessia://localhost");
        let response = call(
            &bridge,
            req("GET", "tessia://localhost/api/%ff%fe")
                .body(Vec::new())
                .unwrap(),
        );
        assert_eq!(response.status(), 400);
    }

    #[test]
    fn api_methods_are_not_restricted_by_the_bridge() {
        // The router decides 404/405 for /api; the bridge forwards every method.
        let bridge = Bridge::new(ui(), echo_api(), "tessia://localhost");
        for method in ["GET", "POST", "PUT", "DELETE", "OPTIONS"] {
            let response = call(
                &bridge,
                req(method, "tessia://localhost/api")
                    .body(Vec::new())
                    .unwrap(),
            );
            assert_eq!(body_json(&response)["method"], method);
        }
    }
}
