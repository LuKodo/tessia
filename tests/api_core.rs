//! The API core: dispatch fallbacks, headers, same-origin guard.

#[path = "api_support.rs"]
mod support;

use support::*;
use tessia::api::Request;

const ALL_ROUTES: [(&str, &str); 6] = [
    ("GET", "/api/health"),
    ("GET", "/api/parameters"),
    ("PUT", "/api/parameters"),
    ("GET", "/api/jobs/current"),
    ("GET", "/api/nope"),
    ("DELETE", "/api/parameters"),
];

#[test]
fn unknown_paths_are_404_with_the_json_body() {
    let h = Harness::new();
    for path in [
        "/api/nope",
        "/",
        "/api",
        "/api/cargues/x",
        "/api/health/",
        "//api/health",
    ] {
        let resp = h.get(path);
        assert_eq!(resp.status, 404, "{path}");
        assert_eq!(error_of(&resp), "ruta no encontrada", "{path}");
        assert_eq!(
            resp.header("Content-Type"),
            Some("application/json; charset=utf-8")
        );
        assert!(resp.body.ends_with(b"\n"));
    }
}

#[test]
fn a_known_path_with_another_method_is_405_with_allow() {
    let h = Harness::new();
    let resp = h.send(Request::new("POST", "/api/health"));
    assert_eq!(resp.status, 405);
    assert_eq!(error_of(&resp), "método no permitido");
    assert_eq!(resp.header("Allow"), Some("GET"));

    let resp = h.send(Request::new("DELETE", "/api/parameters"));
    assert_eq!(resp.status, 405);
    assert_eq!(resp.header("Allow"), Some("GET, PUT"));

    let resp = h.send(Request::new("GET", "/api/jobs/cancel"));
    assert_eq!((resp.status, resp.header("Allow")), (405, Some("POST")));
}

#[test]
fn a_method_mismatch_never_reaches_the_origin_guard() {
    let h = Harness::new();
    let resp = h.send(
        Request::new("DELETE", "/api/parameters").with_header("Origin", "https://evil.example"),
    );
    assert_eq!(resp.status, 405);
}

#[test]
fn every_answer_is_no_store_and_a_plain_answer() {
    let h = Harness::new();
    for (method, path) in ALL_ROUTES {
        let resp = h.send(
            Request::new(method, path)
                .with_header("If-None-Match", "\"abc\"")
                .with_header("Accept-Encoding", "gzip"),
        );
        assert_eq!(
            resp.header("Cache-Control"),
            Some("no-store"),
            "{method} {path}"
        );
        assert_eq!(
            resp.headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("cache-control"))
                .count(),
            1
        );
        assert_ne!(resp.status, 304);
        for banned in ["ETag", "Content-Encoding", "Vary"] {
            assert!(resp.header(banned).is_none(), "{banned} on {method} {path}");
        }
    }
}

#[test]
fn state_changing_routes_reject_other_origins_before_the_handler() {
    let h = Harness::new();
    let cross = [
        Request::new("PUT", "/api/parameters").with_header("Origin", "https://evil.example"),
        Request::new("PUT", "/api/parameters").with_header("Sec-Fetch-Site", "cross-site"),
        Request::new("POST", "/api/convenio/upload").with_header("Origin", "null"),
        Request::new("POST", "/api/run").with_header("Origin", "http://localhost:1234"),
        Request::new("POST", "/api/cargues/C1/reintentar")
            .with_header("Sec-Fetch-Site", "cross-site"),
    ];
    for req in cross {
        let label = format!("{} {}", req.method, req.path);
        let resp = h.send(req);
        assert_eq!(resp.status, 403, "{label}");
        assert_eq!(
            error_of(&resp),
            "solicitud de otro origen no permitida",
            "{label}"
        );
        assert_eq!(resp.header("Cache-Control"), Some("no-store"));
    }
}

#[test]
fn app_origins_and_header_less_requests_reach_the_handler() {
    let h = Harness::new();
    for origin in ["http://wails.localhost", "wails://wails"] {
        let resp = h.send(Request::new("PUT", "/api/parameters").with_header("Origin", origin));
        assert_ne!(resp.status, 403, "{origin}");
    }
    assert_ne!(h.send(Request::new("PUT", "/api/parameters")).status, 403);
}

#[test]
fn the_shell_can_register_its_origin() {
    let h = Harness::new();
    let req = || Request::new("PUT", "/api/parameters").with_header("Origin", "tessia://localhost");
    assert_eq!(h.send(req()).status, 403);
    h.state.allow_origin("tessia://localhost");
    assert_ne!(h.send(req()).status, 403);
}

#[test]
fn read_routes_are_open_to_any_origin() {
    let h = Harness::new();
    let resp = h.send(
        Request::new("GET", "/api/parameters")
            .with_header("Origin", "https://evil.example")
            .with_header("Sec-Fetch-Site", "cross-site"),
    );
    assert_ne!(resp.status, 403);
}
