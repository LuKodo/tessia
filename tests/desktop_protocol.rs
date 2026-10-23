//! The desktop protocol bridge against the real in-process API (`ApiState`) on a
//! throwaway database: what the webview would get for `/api/*` and for the UI.
//! The window itself cannot open in CI; everything up to it is exercised here.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use tessia::api::ApiState;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::desktop::Backend;
use tessia::desktop::assets::MemoryAssets;
use tessia::jobs::JobRunner;

type Answer = http::Response<Cow<'static, [u8]>>;

struct Fixture {
    backend: Backend,
    _db: TestDb,
}

fn fixture() -> Fixture {
    let db = TestDb::new();
    let ctx = AppContext::for_tests(&db);
    let jobs = Arc::new(JobRunner::with_history_dir(ctx.clone(), None));
    let state = ApiState::new(ctx, db.pool(), jobs);
    let ui = MemoryAssets::new()
        .with("index.html", "<!doctype html><title>Tessia</title>")
        .with("assets/index-abc.js", "export {}");
    Fixture {
        backend: Backend::from_state(state, Arc::new(ui)),
        _db: db,
    }
}

fn ask(backend: &Backend, method: &str, uri: &str) -> Option<Answer> {
    let request = http::Request::builder()
        .method(method)
        .uri(uri)
        .body(Vec::new())
        .expect("request");
    let (tx, rx) = mpsc::channel();
    backend.bridge().handle(
        request,
        Box::new(move |answer| {
            let _ = tx.send(answer);
        }),
    );
    rx.recv_timeout(Duration::from_secs(10)).ok()
}

fn get(backend: &Backend, path: &str) -> Answer {
    ask(backend, "GET", &format!("tessia://localhost{path}")).expect("an answer")
}

fn json(answer: &Answer) -> serde_json::Value {
    serde_json::from_slice(answer.body()).expect("json body")
}

#[test]
fn api_paths_reach_the_router_and_keep_its_answers() {
    let f = fixture();

    let unknown = get(&f.backend, "/api/no-such-route");
    assert_eq!(unknown.status(), 404);
    assert_eq!(
        json(&unknown),
        serde_json::json!({"error": "ruta no encontrada"})
    );
    assert_eq!(unknown.headers()["cache-control"], "no-store");
    assert!(
        unknown.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );

    // `/api` itself is the API's too (a 404 from its router, not the UI shell).
    let bare = get(&f.backend, "/api");
    assert_eq!(bare.status(), 404);
    assert_eq!(
        json(&bare),
        serde_json::json!({"error": "ruta no encontrada"})
    );

    // A wrong method keeps the router's 405 and its Allow header.
    let wrong = ask(&f.backend, "DELETE", "tessia://localhost/api/health").expect("an answer");
    assert_eq!(wrong.status(), 405);
    assert_eq!(
        json(&wrong),
        serde_json::json!({"error": "método no permitido"})
    );
    assert!(wrong.headers()["allow"].to_str().unwrap().contains("GET"));
}

#[test]
fn the_ui_is_served_next_to_the_api() {
    let f = fixture();

    let index = get(&f.backend, "/");
    assert_eq!(index.status(), 200);
    assert_eq!(index.headers()["content-type"], "text/html; charset=utf-8");
    assert!(String::from_utf8_lossy(index.body()).contains("<title>Tessia</title>"));

    // A client-side route falls back to the shell; a missing file is a 404.
    assert_eq!(get(&f.backend, "/ejecuciones").body(), index.body());
    assert_eq!(get(&f.backend, "/assets/missing.js").status(), 404);
    assert_eq!(get(&f.backend, "/assets/index-abc.js").status(), 200);

    // Static content is read-only.
    let post = ask(&f.backend, "POST", "tessia://localhost/").expect("an answer");
    assert_eq!(post.status(), 405);
    assert_eq!(post.headers()["allow"], "GET, HEAD");
}

#[test]
fn many_concurrent_requests_are_all_answered() {
    let f = fixture();
    let backend = &f.backend;
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..16)
            .map(|n| {
                scope.spawn(move || {
                    for _ in 0..5 {
                        let path = if n % 2 == 0 { "/api/nope" } else { "/" };
                        let answer = get(backend, path);
                        assert_eq!(answer.status(), if n % 2 == 0 { 404 } else { 200 });
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("no panic");
        }
    });
}

#[test]
fn a_closed_backend_answers_no_more_api_requests() {
    let f = fixture();
    let bridge = f.backend.bridge();
    f.backend.close();

    let (tx, rx) = mpsc::channel();
    let request = http::Request::builder()
        .uri("tessia://localhost/api/health")
        .body(Vec::new())
        .expect("request");
    bridge.handle(
        request,
        Box::new(move |answer| {
            let _ = tx.send(answer);
        }),
    );
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}
